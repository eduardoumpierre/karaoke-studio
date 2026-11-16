import os
import sqlite3
import subprocess

import numpy as np
import pytest
from fastapi.testclient import TestClient

from app.config import TOKEN_HEADER, Settings
from app.main import create_app
from app.services.pitch import pitch_filter

from .conftest import TOKEN
from .test_library import import_, make_audio, samples  # noqa: F401  (fixture)

H = {TOKEN_HEADER: TOKEN}


def dominant_freq(path) -> float:
    raw = subprocess.run(
        [
            "ffmpeg",
            "-loglevel",
            "error",
            "-i",
            str(path),
            "-f",
            "f32le",
            "-ac",
            "1",
            "-ar",
            "44100",
            "-",
        ],
        capture_output=True,
        check=True,
    ).stdout
    y = np.frombuffer(raw, dtype=np.float32)
    mid = y[len(y) // 4 : 3 * len(y) // 4]
    spectrum = np.abs(np.fft.rfft(mid * np.hanning(len(mid))))
    return float(np.fft.rfftfreq(len(mid), 1 / 44100)[np.argmax(spectrum)])


@pytest.fixture
def env(tmp_path, samples):  # noqa: F811
    settings = Settings(token=TOKEN, data_dir=tmp_path)
    with TestClient(create_app(settings)) as c:
        body = import_(c, samples["wav"]).json()  # 523 Hz
        sid = body["id"]
        stored = c.get(f"/songs/{sid}/audio", headers=H).json()["original"]
        yield c, sid, tmp_path, os.path.dirname(stored)


def make_stems(song_dir):
    stems = os.path.join(song_dir, "stems")
    os.makedirs(stems, exist_ok=True)
    make_audio(__import__("pathlib").Path(stems) / "vocals.flac", freq=300, seconds=4)
    make_audio(__import__("pathlib").Path(stems) / "instrumental.flac", freq=440, seconds=4)


def test_filter_string():
    assert "pitch=1.122462" in pitch_filter(2, voice=False)
    assert "formant=preserved" in pitch_filter(2, voice=True)
    assert "formant" not in pitch_filter(-2, voice=False)
    assert "pitch=0.500000" in pitch_filter(-12, voice=False)


def test_requires_token_and_unknown_song(env):
    c, sid, *_ = env
    assert c.get(f"/songs/{sid}/pitch").status_code == 401
    assert c.get("/songs/999/pitch", headers=H).status_code == 404
    assert c.put("/songs/999/pitch", json={"semitones": 1}, headers=H).status_code == 404


def test_default_and_zero_is_plain_audio(env):
    c, sid, *_ = env
    assert c.get(f"/songs/{sid}/pitch", headers=H).json() == {
        "semitones": 0,
        "rendered": [],
        "cache_bytes": 0,
    }
    plain = c.get(f"/songs/{sid}/audio", headers=H).json()
    r = c.put(f"/songs/{sid}/pitch", json={"semitones": 0}, headers=H).json()
    assert r["audio"] == plain and r["pitch"]["rendered"] == []


def test_unseparated_song_shifts_original(env):
    c, sid, *_ = env
    r = c.put(f"/songs/{sid}/pitch", json={"semitones": 3}, headers=H)
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["audio"]["vocals"] is None and body["audio"]["instrumental"] is None
    assert body["audio"]["original"].endswith("original.flac")
    assert dominant_freq(body["audio"]["original"]) == pytest.approx(523 * 2 ** (3 / 12), rel=0.02)
    assert body["pitch"]["semitones"] == 3 and body["pitch"]["rendered"] == [3]
    assert body["pitch"]["cache_bytes"] > 0


def test_separated_song_shifts_both_stems(env):
    c, sid, _, song_dir = env
    make_stems(song_dir)
    body = c.put(f"/songs/{sid}/pitch", json={"semitones": -2}, headers=H).json()
    audio = body["audio"]
    assert audio["vocals"].endswith("vocals.flac") and "pitch" in audio["vocals"]
    assert dominant_freq(audio["vocals"]) == pytest.approx(300 * 2 ** (-2 / 12), rel=0.02)
    assert dominant_freq(audio["instrumental"]) == pytest.approx(440 * 2 ** (-2 / 12), rel=0.02)
    # a música original continua intacta
    assert dominant_freq(audio["original"]) == pytest.approx(523, rel=0.02)


def test_cache_reuse_and_rerender_when_source_is_newer(env):
    c, sid, _, song_dir = env
    make_stems(song_dir)
    a = c.put(f"/songs/{sid}/pitch", json={"semitones": 2}, headers=H).json()["audio"]
    inst = a["instrumental"]
    first = os.stat(inst).st_mtime_ns
    c.put(f"/songs/{sid}/pitch", json={"semitones": 2}, headers=H)
    assert os.stat(inst).st_mtime_ns == first  # reaproveitou
    src = os.path.join(song_dir, "stems", "instrumental.flac")
    os.utime(src, (first / 1e9 + 100, first / 1e9 + 100))  # separada de novo
    c.put(f"/songs/{sid}/pitch", json={"semitones": 2}, headers=H)
    assert os.stat(inst).st_mtime_ns != first


def test_choice_persists_and_cache_can_be_cleared(env):
    c, sid, _, song_dir = env
    make_stems(song_dir)
    c.put(f"/songs/{sid}/pitch", json={"semitones": 2}, headers=H)
    c.put(f"/songs/{sid}/pitch", json={"semitones": -1}, headers=H)
    p = c.get(f"/songs/{sid}/pitch", headers=H).json()
    assert p["semitones"] == -1 and p["rendered"] == [-1, 2]
    cleared = c.delete(f"/songs/{sid}/pitch/cache", headers=H).json()
    assert cleared == {"semitones": -1, "rendered": [], "cache_bytes": 0}
    # volta a renderizar sob demanda
    again = c.put(f"/songs/{sid}/pitch", json={"semitones": -1}, headers=H).json()
    assert os.path.isfile(again["audio"]["instrumental"])


def test_validation_and_busy(env):
    c, sid, tmp_path, _ = env
    for n in (7, -7):
        assert c.put(f"/songs/{sid}/pitch", json={"semitones": n}, headers=H).status_code == 422
    with sqlite3.connect(tmp_path / "library.db") as db:
        db.execute(
            "INSERT INTO jobs (song_id, kind, status) VALUES (?, 'separate', 'running')", (sid,)
        )
    r = c.put(f"/songs/{sid}/pitch", json={"semitones": 1}, headers=H)
    assert r.status_code == 409 and r.json()["code"] == "song_busy"
    # tocar no tom 0 / consultar continua permitido
    assert c.get(f"/songs/{sid}/pitch", headers=H).status_code == 200


def test_delete_song_removes_pitch_data(env):
    c, sid, tmp_path, song_dir = env
    c.put(f"/songs/{sid}/pitch", json={"semitones": 1}, headers=H)
    assert c.delete(f"/songs/{sid}", headers=H).status_code == 204
    assert not os.path.exists(song_dir)
    with sqlite3.connect(tmp_path / "library.db") as db:
        assert db.execute("SELECT count(*) FROM song_pitch").fetchone()[0] == 0


def test_newer_pitch_request_supersedes_older(env, monkeypatch):
    import threading

    from app.services import pitch as pitch_mod

    c, sid = env[0], env[1]
    first_started = threading.Event()

    def fake_render(src, dst, semitones, voice, cancel=None):
        if semitones == 1:
            first_started.set()
            assert cancel.wait(5), "o pedido antigo deveria ser cancelado"
            raise pitch_mod.Cancelled
        dst.write_bytes(b"x")

    monkeypatch.setattr(pitch_mod, "render_shifted", fake_render)
    results = {}

    def first():
        results["first"] = c.put(f"/songs/{sid}/pitch", json={"semitones": 1}, headers=H)

    t = threading.Thread(target=first)
    t.start()
    assert first_started.wait(5)
    second = c.put(f"/songs/{sid}/pitch", json={"semitones": 2}, headers=H)
    t.join(5)
    assert second.status_code == 200
    assert results["first"].status_code == 409
