import numpy as np
import pytest
from fastapi.testclient import TestClient

from app.config import TOKEN_HEADER, Settings
from app.main import create_app
from app.services.analyzer import (
    MAJOR_PROFILE,
    MINOR_PROFILE,
    AnalysisResult,
    LibrosaAnalyzer,
    estimate_key,
    fold_bpm,
)

from .conftest import TOKEN
from .test_library import import_, samples  # noqa: F401  (fixture)

H = {TOKEN_HEADER: TOKEN}


# ----------------------------------------------------------------------------- funções puras
@pytest.mark.parametrize("tonic", range(12))
@pytest.mark.parametrize(("mode", "profile"), [("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)])
def test_estimate_key_recovers_rotated_profiles(tonic, mode, profile):
    got = estimate_key(np.roll(profile, tonic))
    assert got[0] == tonic and got[1] == mode and got[2] > 0.99


def test_estimate_key_degenerate_input():
    assert estimate_key(np.zeros(12)) == (0, "major", 0.0)
    assert estimate_key(np.ones(5)) == (0, "major", 0.0)


def test_estimate_key_noise_tolerant():
    rng = np.random.default_rng(1)
    chroma = np.roll(MINOR_PROFILE, 9) + rng.normal(0, 0.4, 12)  # Lá menor com ruído
    assert estimate_key(chroma)[:2] == (9, "minor")


@pytest.mark.parametrize(
    ("raw", "folded"), [(60, 120), (240, 120), (35, 70), (100, 100), (180, 180), (181, 90.5)]
)
def test_fold_bpm(raw, folded):
    assert fold_bpm(raw) == pytest.approx(folded)


# ----------------------------------------------------------------------------- Librosa real
def test_librosa_analyzer_on_synthetic_music(tmp_path):
    librosa = pytest.importorskip("librosa")
    sf = pytest.importorskip("soundfile")
    import shutil

    if shutil.which("ffmpeg") is None:
        pytest.skip("ffmpeg ausente")
    sr = 22050
    bpm, seconds = 120, 40
    y = librosa.clicks(times=np.arange(0, seconds, 60 / bpm), sr=sr, length=seconds * sr) * 0.5
    t = np.arange(seconds * sr) / sr
    for midi in (60, 64, 67):  # tríade de Dó maior
        y += 0.2 * np.sin(2 * np.pi * librosa.midi_to_hz(midi) * t)
    path = tmp_path / "song.wav"
    sf.write(path, y.astype("float32"), sr)
    result = LibrosaAnalyzer().analyze(path, seconds)
    assert result.bpm == pytest.approx(120, abs=3) or result.bpm == pytest.approx(60, abs=2) * 2
    assert (result.tonic, result.mode) == (0, "major")


# ----------------------------------------------------------------------------- API
class FakeAnalyzer:
    def __init__(self):
        self.calls = 0
        self.fail = False

    def analyze(self, path, duration, progress=None, cancel=None):
        self.calls += 1
        if progress:
            progress(0.5, "Detectando o andamento")
        if self.fail:
            raise RuntimeError("quebrou")
        return AnalysisResult(bpm=100.0 + self.calls, tonic=9, mode="minor", confidence=0.8)


def make(tmp_path, fake):
    return TestClient(create_app(Settings(token=TOKEN, data_dir=tmp_path), analyzer=fake))


def run_and_wait(c, sid, force=False, timeout=10):
    """Enfileira a análise (202) e espera a fila terminar; devolve o GET final."""
    import time

    r = c.post(f"/songs/{sid}/analysis/run{'?force=true' if force else ''}", headers=H)
    assert r.status_code == 202, r.text
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        body = c.get(f"/songs/{sid}/analysis", headers=H).json()
        if not body["running"]:
            return body
        time.sleep(0.02)
    raise AssertionError("a análise não terminou")


def song(c, samples):  # noqa: F811
    return import_(c, samples["mp3"]).json()["id"]


def test_requires_token_and_unknown_song(tmp_path):
    with make(tmp_path, FakeAnalyzer()) as c:
        assert c.get("/songs/1/analysis").status_code == 401
        assert c.get("/songs/99/analysis", headers=H).status_code == 404
        assert c.post("/songs/99/analysis/run", headers=H).status_code == 404
        assert c.put("/songs/99/analysis", json={"bpm": 90}, headers=H).status_code == 404


def test_run_and_get(tmp_path, samples):  # noqa: F811
    fake = FakeAnalyzer()
    with make(tmp_path, fake) as c:
        sid = song(c, samples)
        assert c.get(f"/songs/{sid}/analysis", headers=H).json()["status"] == "none"
        body = run_and_wait(c, sid)
        assert body["analyzed_at"]
        assert body["running"] is False and body["progress"] is None
        assert {k: body[k] for k in ("status", "bpm", "key_tonic", "key_mode", "key_manual")} == {
            "status": "done",
            "bpm": 101.0,
            "key_tonic": 9,
            "key_mode": "minor",
            "key_manual": False,
        }
        assert c.get(f"/songs/{sid}/analysis", headers=H).json()["bpm"] == 101.0


def test_manual_edits_survive_rerun_unless_forced(tmp_path, samples):  # noqa: F811
    fake = FakeAnalyzer()
    with make(tmp_path, fake) as c:
        sid = song(c, samples)
        run_and_wait(c, sid)
        r = c.put(f"/songs/{sid}/analysis", json={"bpm": 87.46}, headers=H).json()
        assert r["bpm"] == 87.5 and r["bpm_manual"] and not r["key_manual"]
        r = c.put(f"/songs/{sid}/analysis", json={"key_tonic": 0, "key_mode": "major"}, headers=H)
        r = r.json()
        assert (r["key_tonic"], r["key_mode"], r["key_manual"]) == (0, "major", True)
        assert r["key_confidence"] is None
        r = run_and_wait(c, sid)
        assert r["bpm"] == 87.5 and r["key_tonic"] == 0  # preservados
        r = run_and_wait(c, sid, force=True)
        assert r["bpm"] == 103.0 and r["key_tonic"] == 9 and not r["bpm_manual"]


def test_edit_only_mode_keeps_tonic_and_edit_before_analysis(tmp_path, samples):  # noqa: F811
    with make(tmp_path, FakeAnalyzer()) as c:
        sid = song(c, samples)
        r = c.put(f"/songs/{sid}/analysis", json={"key_mode": "minor"}, headers=H).json()
        assert (r["key_tonic"], r["key_mode"], r["key_manual"]) == (0, "minor", True)
        assert r["bpm"] is None
        r = c.put(f"/songs/{sid}/analysis", json={"key_tonic": 7}, headers=H).json()
        assert (r["key_tonic"], r["key_mode"]) == (7, "minor")


def test_validation_and_failures(tmp_path, samples):  # noqa: F811
    fake = FakeAnalyzer()
    with make(tmp_path, fake) as c:
        sid = song(c, samples)
        for body in ({"bpm": 5}, {"bpm": 999}, {"key_tonic": 12}, {"key_mode": "dorian"}):
            assert c.put(f"/songs/{sid}/analysis", json=body, headers=H).status_code == 422
        fake.fail = True
        body = run_and_wait(c, sid)
        assert body["status"] == "none" and body["running"] is False
        assert "quebrou" in body["error"]


def test_cascade_on_song_delete(tmp_path, samples):  # noqa: F811
    import sqlite3

    with make(tmp_path, FakeAnalyzer()) as c:
        sid = song(c, samples)
        run_and_wait(c, sid)
        assert c.delete(f"/songs/{sid}", headers=H).status_code == 204
        with sqlite3.connect(tmp_path / "library.db") as db:
            assert db.execute("SELECT count(*) FROM analysis").fetchone()[0] == 0


def test_analysis_runs_in_background_queue(tmp_path, samples):  # noqa: F811
    import threading

    started, release = threading.Event(), threading.Event()

    class Slow(FakeAnalyzer):
        def analyze(self, path, duration, progress=None, cancel=None):
            progress(0.42, "Detectando o andamento")
            started.set()
            release.wait(5)
            return super().analyze(path, duration, progress, cancel)

    with make(tmp_path, Slow()) as c:
        sid = song(c, samples)
        # o POST responde na hora, sem esperar a análise
        r = c.post(f"/songs/{sid}/analysis/run", headers=H)
        assert r.status_code == 202
        assert started.wait(5)
        mid = c.get(f"/songs/{sid}/analysis", headers=H).json()
        assert mid["status"] == "none" and mid["running"] is True and mid["queued"] is False
        assert mid["progress"] == 0.42 and mid["stage"] == "Detectando o andamento"
        # aparece na lista de tarefas, com o título da música
        jobs = c.get("/jobs", headers=H).json()
        assert [(j["kind"], j["status"], j["song_title"]) for j in jobs][0][:2] == (
            "analyze",
            "running",
        )
        # um segundo pedido não cria outra tarefa
        c.post(f"/songs/{sid}/analysis/run", headers=H)
        assert len(c.get("/jobs", headers=H).json()) == 1
        release.set()
        import time

        done = None
        for _ in range(100):
            done = c.get(f"/songs/{sid}/analysis", headers=H).json()
            if not done["running"]:
                break
            time.sleep(0.02)
        assert done["status"] == "done" and done["running"] is False


def test_queued_analysis_can_be_cancelled_and_does_not_block_pitch(tmp_path, samples):  # noqa: F811
    import threading

    started, release = threading.Event(), threading.Event()

    class Slow(FakeAnalyzer):
        def analyze(self, path, duration, progress=None, cancel=None):
            started.set()
            release.wait(5)
            return super().analyze(path, duration, progress, cancel)

    with make(tmp_path, Slow()) as c:
        a = song(c, samples)
        b = import_(c, samples["wav"]).json()["id"]
        c.post(f"/songs/{a}/analysis/run", headers=H)
        assert started.wait(5)
        c.post(f"/songs/{b}/analysis/run", headers=H)  # fica na fila atrás da primeira
        queued = c.get(f"/songs/{b}/analysis", headers=H).json()
        assert queued["running"] and queued["queued"] and queued["stage"] == "Na fila"
        job_b = next(j for j in c.get("/jobs", headers=H).json() if j["song_id"] == b)
        assert c.post(f"/jobs/{job_b['id']}/cancel", headers=H).json()["status"] == "cancelled"
        assert c.get(f"/songs/{b}/analysis", headers=H).json()["running"] is False
        # analisar não impede trocar de tom (só a separação impede)
        assert c.put(f"/songs/{a}/pitch", json={"semitones": 0}, headers=H).status_code == 200
        release.set()


def test_stage_progress_is_monotonic_and_bounded():
    from app.services.analyzer import STAGES, stage_progress

    for i, (label, start, end, _) in enumerate(STAGES):
        a, la = stage_progress(i, 0.0)
        b, _ = stage_progress(i, 5.0)
        c, _ = stage_progress(i, 500.0)
        assert la == label and a == start
        assert start <= a < b < c < end


# ----------------------------------------------------------------------------- processo de análise
def test_worker_failure_reports_the_error_and_cancel_kills_it(tmp_path):
    import threading
    import time

    from app.services.analyzer import WorkerFailed
    from app.services.audio_tools import Cancelled

    analyzer = LibrosaAnalyzer(tmp_path / "numba")
    try:
        with pytest.raises(WorkerFailed, match="sem resultado"):
            analyzer._run([str(tmp_path / "nao-existe.wav")], threading.Event())
        cancel = threading.Event()
        threading.Timer(0.3, cancel.set).start()
        started = time.monotonic()
        with pytest.raises(Cancelled):
            analyzer._run(["--warmup"], cancel)  # compila numba: dá tempo de cancelar
        assert time.monotonic() - started < 10
        assert not analyzer._procs
    finally:
        analyzer.close()


def test_worker_streams_stages_and_result(tmp_path):
    librosa = pytest.importorskip("librosa")
    sf = pytest.importorskip("soundfile")
    import threading

    sr = 22050
    y = librosa.clicks(times=np.arange(0, 20, 0.5), sr=sr, length=20 * sr) * 0.5
    wav = tmp_path / "a.wav"
    sf.write(wav, y.astype("float32"), sr)
    stages: list[int] = []
    analyzer = LibrosaAnalyzer(tmp_path / "numba")
    out = analyzer._run([str(wav)], threading.Event(), stages.append)
    assert stages == [1, 2, 3]
    assert set(out) >= {"bpm", "tonic", "mode", "confidence", "timings"}
