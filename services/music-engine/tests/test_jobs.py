import shutil
import threading
import time
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.config import TOKEN_HEADER, Settings
from app.main import create_app
from app.services.audio_tools import AudioToolError, Cancelled
from app.services.separator import Stems, parse_progress

from .conftest import TOKEN
from .test_library import import_, samples  # noqa: F401  (fixture)

H = {TOKEN_HEADER: TOKEN}


class FakeSeparator:
    """Copia o WAV normalizado como se fossem as duas stems, emitindo progresso."""

    def __init__(self, hold: threading.Event | None = None, fail: str | None = None) -> None:
        self.hold = hold
        self.fail = fail
        self.started = threading.Event()
        self.inputs: list[tuple[int, int]] = []

    def unavailable_reason(self) -> str | None:
        return None

    def separate(self, wav, out_dir, on_progress, cancel):
        import struct

        # Cabeçalho WAV (o módulo wave não lê float): canais no byte 22, taxa no 24.
        channels, rate = struct.unpack("<HI", wav.read_bytes()[22:28])
        self.inputs.append((rate, channels))
        self.started.set()
        on_progress(0.3)
        if self.hold is not None:
            while not self.hold.is_set():
                if cancel.is_set():
                    raise Cancelled
                time.sleep(0.01)
        if self.fail:
            raise AudioToolError(self.fail)
        out_dir.mkdir(parents=True)
        vocals, inst = out_dir / "vocals.wav", out_dir / "no_vocals.wav"
        shutil.copyfile(wav, vocals)
        shutil.copyfile(wav, inst)
        on_progress(1.0)
        return Stems(vocals, inst)


class Unavailable(FakeSeparator):
    def unavailable_reason(self):
        return "Demucs ausente"


def make_client(tmp_path, separator):
    app = create_app(Settings(token=TOKEN, data_dir=tmp_path), separator=separator)
    return TestClient(app)


def wait_for(client, job_id, statuses, timeout=15):
    end = time.time() + timeout
    while time.time() < end:
        job = client.get(f"/jobs/{job_id}", headers=H).json()
        if job["status"] in statuses:
            return job
        time.sleep(0.02)
    raise AssertionError(f"job {job_id} não chegou a {statuses}: {job}")


def new_song(client, samples, key="mp3"):  # noqa: F811
    r = import_(client, samples[key])
    assert r.status_code == 201, r.text
    return r.json()


def test_parse_progress():
    assert parse_progress(" 45%|████▌     | 12.6/27.9 [00:10<00:12]") == 0.45
    assert parse_progress("100%|██████████| 28/28") == 1.0
    assert parse_progress("Downloading: https://x/y.th") is None
    assert parse_progress("\x1b[A 7%|▋ | 1/14") == 0.07


def test_requires_token(tmp_path):
    with make_client(tmp_path, FakeSeparator()) as c:
        assert c.get("/jobs").status_code == 401
        assert c.post("/songs/1/separate").status_code == 401
        assert c.post("/jobs/1/cancel").status_code == 401


def test_separate_happy_path(tmp_path, samples):  # noqa: F811
    sep = FakeSeparator()
    with make_client(tmp_path, sep) as c:
        song = new_song(c, samples)
        r = c.post(f"/songs/{song['id']}/separate", headers=H)
        assert r.status_code == 202, r.text
        job = wait_for(c, r.json()["id"], {"done", "failed"})
        assert job["status"] == "done", job
        assert job["progress"] == 1 and job["error"] is None
        assert c.get(f"/songs/{song['id']}", headers=H).json()["status"] == "separated"
        stems = next((tmp_path / "songs").glob("*/stems"))
        assert sorted(p.name for p in stems.iterdir()) == ["instrumental.flac", "vocals.flac"]
        assert all(p.read_bytes()[:4] == b"fLaC" for p in stems.iterdir())
        assert sep.inputs == [(44100, 2)]  # mp3 mono de entrada foi normalizado
        assert not (tmp_path / "tmp" / f"job-{job['id']}").exists()


def test_already_separated_and_force(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, FakeSeparator()) as c:
        song = new_song(c, samples)
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        wait_for(c, job["id"], {"done"})
        r = c.post(f"/songs/{song['id']}/separate", headers=H)
        assert r.status_code == 409 and r.json()["code"] == "already_separated"
        r = c.post(f"/songs/{song['id']}/separate?force=true", headers=H)
        assert r.status_code == 202
        wait_for(c, r.json()["id"], {"done"})


def test_unknown_song_and_unavailable_separator(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, Unavailable()) as c:
        assert c.post("/songs/999/separate", headers=H).status_code == 404
        song = new_song(c, samples)
        r = c.post(f"/songs/{song['id']}/separate", headers=H)
        assert r.status_code == 503
        assert r.json()["code"] == "separator_unavailable"
        assert c.get("/jobs", headers=H).json() == []


def test_failure_is_reported(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, FakeSeparator(fail="sem memória na GPU")) as c:
        song = new_song(c, samples)
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        done = wait_for(c, job["id"], {"failed"})
        assert "sem memória na GPU" in done["error"]
        assert c.get(f"/songs/{song['id']}", headers=H).json()["status"] == "imported"
        assert not (tmp_path / "tmp" / f"job-{job['id']}").exists()


def test_corrupt_original_fails_cleanly(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, FakeSeparator()) as c:
        song = new_song(c, samples)
        original = next((tmp_path / "songs").glob("*/original.mp3"))
        original.write_bytes(b"isto nao e audio")
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        done = wait_for(c, job["id"], {"failed"})
        assert "Normalização" in done["error"]


def test_single_worker_runs_in_order_and_cancels_queued(tmp_path, samples):  # noqa: F811
    hold = threading.Event()
    sep = FakeSeparator(hold=hold)
    with make_client(tmp_path, sep) as c:
        a = new_song(c, samples, "mp3")
        b = new_song(c, samples, "wav")
        d = new_song(c, samples, "flac")
        ja = c.post(f"/songs/{a['id']}/separate", headers=H).json()
        assert sep.started.wait(10)
        jb = c.post(f"/songs/{b['id']}/separate", headers=H).json()
        jd = c.post(f"/songs/{d['id']}/separate", headers=H).json()
        assert wait_for(c, ja["id"], {"running"})["progress"] > 0
        assert c.get(f"/jobs/{jb['id']}", headers=H).json()["status"] == "queued"
        assert [j["id"] for j in c.get("/jobs?active=true", headers=H).json()] == [
            jd["id"],
            jb["id"],
            ja["id"],
        ]
        # idempotente: pedir de novo devolve o mesmo job
        again = c.post(f"/songs/{b['id']}/separate", headers=H).json()
        assert again["id"] == jb["id"]
        # cancela o que está na fila
        cancelled = c.post(f"/jobs/{jb['id']}/cancel", headers=H).json()
        assert cancelled["status"] == "cancelled"
        hold.set()
        wait_for(c, ja["id"], {"done"})
        wait_for(c, jd["id"], {"done"})
        assert c.get(f"/jobs/{jb['id']}", headers=H).json()["status"] == "cancelled"
        statuses = {s["id"]: s["status"] for s in c.get("/songs", headers=H).json()}
        assert statuses == {a["id"]: "separated", b["id"]: "imported", d["id"]: "separated"}


def test_cancel_running_job(tmp_path, samples):  # noqa: F811
    sep = FakeSeparator(hold=threading.Event())  # nunca libera: só termina por cancelamento
    with make_client(tmp_path, sep) as c:
        song = new_song(c, samples)
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        assert sep.started.wait(10)
        r = c.post(f"/jobs/{job['id']}/cancel", headers=H)
        assert r.status_code == 200
        assert wait_for(c, job["id"], {"cancelled"})["status"] == "cancelled"
        assert c.get(f"/songs/{song['id']}", headers=H).json()["status"] == "imported"
        assert not any((tmp_path / "songs").glob("*/stems"))
        again = c.post(f"/jobs/{job['id']}/cancel", headers=H)
        assert again.status_code == 409 and again.json()["code"] == "job_finished"
        assert c.post("/jobs/999/cancel", headers=H).status_code == 404


def test_cannot_delete_song_while_processing(tmp_path, samples):  # noqa: F811
    sep = FakeSeparator(hold=threading.Event())
    with make_client(tmp_path, sep) as c:
        song = new_song(c, samples)
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        assert sep.started.wait(10)
        r = c.delete(f"/songs/{song['id']}", headers=H)
        assert r.status_code == 409 and r.json()["code"] == "song_busy"
        c.post(f"/jobs/{job['id']}/cancel", headers=H)
        wait_for(c, job["id"], {"cancelled"})
        assert c.delete(f"/songs/{song['id']}", headers=H).status_code == 204


def test_delete_removes_stems(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, FakeSeparator()) as c:
        song = new_song(c, samples)
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        wait_for(c, job["id"], {"done"})
        assert c.delete(f"/songs/{song['id']}", headers=H).status_code == 204
        assert not any((tmp_path / "songs").iterdir())


def test_restart_marks_running_jobs_failed_and_runs_queued(tmp_path, samples):  # noqa: F811
    hold = threading.Event()
    sep = FakeSeparator(hold=hold)
    c = make_client(tmp_path, sep)
    c.__enter__()
    song = new_song(c, samples)
    other = new_song(c, samples, "wav")
    running = c.post(f"/songs/{song['id']}/separate", headers=H).json()
    assert sep.started.wait(10)
    queued = c.post(f"/songs/{other['id']}/separate", headers=H).json()
    # Simula uma queda: o banco guarda os estados como estavam, sem passar pelo desligamento.
    import sqlite3

    snap = tmp_path / "snap"
    snap.mkdir()
    src = sqlite3.connect(tmp_path / "library.db")
    dst = sqlite3.connect(snap / "library.db")
    src.backup(dst)
    src.close()
    dst.close()
    shutil.copytree(tmp_path / "songs", snap / "songs")
    c.__exit__(None, None, None)

    with make_client(snap, FakeSeparator()) as c2:
        j1 = wait_for(c2, running["id"], {"failed"})
        assert "Interrompido" in j1["error"]
        assert wait_for(c2, queued["id"], {"done"})["status"] == "done"


@pytest.mark.parametrize("path", ["/jobs", "/jobs?active=true"])
def test_list_empty(tmp_path, path):
    with make_client(tmp_path, FakeSeparator()) as c:
        assert c.get(path, headers=H).json() == []


def test_audio_files_endpoint(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, FakeSeparator()) as c:
        song = new_song(c, samples)
        r = c.get(f"/songs/{song['id']}/audio", headers=H)
        assert r.status_code == 200
        body = r.json()
        assert body["vocals"] is None and body["instrumental"] is None
        assert body["original"].endswith("original.mp3") and Path(body["original"]).is_file()
        job = c.post(f"/songs/{song['id']}/separate", headers=H).json()
        wait_for(c, job["id"], {"done"})
        body = c.get(f"/songs/{song['id']}/audio", headers=H).json()
        assert Path(body["vocals"]).name == "vocals.flac" and Path(body["vocals"]).is_file()
        assert Path(body["instrumental"]).name == "instrumental.flac"
        assert c.get("/songs/999/audio", headers=H).status_code == 404
        assert c.get(f"/songs/{song['id']}/audio").status_code == 401
        Path(body["original"]).unlink()
        assert c.get(f"/songs/{song['id']}/audio", headers=H).status_code == 404
