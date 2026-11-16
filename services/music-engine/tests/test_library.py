import shutil
import subprocess
from pathlib import Path

import pytest

from app.config import TOKEN_HEADER

from .conftest import TOKEN

H = {TOKEN_HEADER: TOKEN}


def make_audio(path: Path, *, freq: int = 440, seconds: float = 2.0, **tags: str) -> Path:
    cmd = ["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi", "-i"]
    cmd += [f"sine=frequency={freq}:duration={seconds}:sample_rate=44100"]
    for k, v in tags.items():
        cmd += ["-metadata", f"{k}={v}"]
    cmd.append(str(path))
    subprocess.run(cmd, check=True)
    return path


@pytest.fixture(scope="session")
def samples(tmp_path_factory) -> dict[str, Path]:
    d = tmp_path_factory.mktemp("samples")
    return {
        "mp3": make_audio(d / "mp3_faixa.mp3", title="Canção Número Um", artist="Banda Teste"),
        "wav": make_audio(d / "som_de_teste.wav", freq=523),
        "flac": make_audio(d / "faixa.flac", freq=660, title="Flac Song", artist="Zeca"),
    }


def import_(client, path: Path):
    return client.post("/songs/import", json={"path": str(path)}, headers=H)


def test_requires_token(client, samples):
    assert client.get("/songs").status_code == 401
    assert client.post("/songs/import", json={"path": str(samples["mp3"])}).status_code == 401


def test_empty_library(client):
    r = client.get("/songs", headers=H)
    assert r.status_code == 200 and r.json() == []


def test_import_mp3_reads_metadata(client, samples):
    r = import_(client, samples["mp3"])
    assert r.status_code == 201, r.text
    song = r.json()
    assert song["title"] == "Canção Número Um"
    assert song["artist"] == "Banda Teste"
    assert song["format"] == "mp3"
    assert song["sample_rate"] == 44100
    assert song["channels"] == 1
    assert 1.9 < song["duration_seconds"] < 2.2
    assert song["status"] == "imported"
    assert "stored_path" not in song and "file_hash" not in song


def test_import_wav_without_tags_uses_filename(client, samples):
    song = import_(client, samples["wav"]).json()
    assert song["title"] == "som de teste"
    assert song["artist"] == ""
    assert song["format"] == "wav"


def test_import_flac(client, samples):
    song = import_(client, samples["flac"]).json()
    assert song["format"] == "flac" and song["title"] == "Flac Song"


def test_file_is_copied_into_the_data_dir(client, samples, tmp_path):
    song = import_(client, samples["mp3"]).json()
    copies = list((client.app.state.settings.data_dir / "songs").glob("*/original.mp3"))
    assert len(copies) == 1
    assert copies[0].stat().st_size == samples["mp3"].stat().st_size
    assert song["size_bytes"] == copies[0].stat().st_size


def test_duplicate_is_rejected_even_if_renamed(client, samples, tmp_path):
    first = import_(client, samples["mp3"]).json()
    renamed = tmp_path / "outro_nome.mp3"
    shutil.copyfile(samples["mp3"], renamed)
    r = import_(client, renamed)
    assert r.status_code == 409
    body = r.json()
    assert body["code"] == "duplicate" and body["song_id"] == first["id"]
    assert len(client.get("/songs", headers=H).json()) == 1


def test_corrupt_file_is_rejected_and_leaves_nothing_behind(client, tmp_path):
    bad = tmp_path / "quebrado.mp3"
    bad.write_bytes(b"isto nao e audio" * 500)
    r = import_(client, bad)
    assert r.status_code == 400 and r.json()["code"] == "invalid_audio"
    assert client.get("/songs", headers=H).json() == []
    assert not (client.app.state.settings.data_dir / "songs").exists() or not any(
        (client.app.state.settings.data_dir / "songs").iterdir()
    )


def test_empty_file_is_rejected(client, tmp_path):
    empty = tmp_path / "vazio.wav"
    empty.write_bytes(b"")
    assert import_(client, empty).status_code == 400


def test_truncated_audio_file_is_rejected_or_readable(client, samples, tmp_path):
    cut = tmp_path / "cortado.flac"
    cut.write_bytes(samples["flac"].read_bytes()[:20])
    assert import_(client, cut).status_code == 400


def test_unsupported_extension(client, tmp_path):
    txt = tmp_path / "nota.txt"
    txt.write_text("oi")
    r = import_(client, txt)
    assert r.status_code == 422 and r.json()["code"] == "unsupported_format"


def test_missing_file(client, tmp_path):
    r = import_(client, tmp_path / "nao_existe.mp3")
    assert r.status_code == 404 and r.json()["code"] == "file_not_found"


def test_directory_is_not_a_file(client, tmp_path):
    assert import_(client, tmp_path).status_code == 404


def test_search_ignores_case_and_accents(client, samples):
    for key in ("mp3", "wav", "flac"):
        import_(client, samples[key])
    titles = lambda q: [s["title"] for s in client.get("/songs", params={"q": q}, headers=H).json()]  # noqa: E731
    assert titles("cancao") == ["Canção Número Um"]
    assert titles("NUMERO") == ["Canção Número Um"]
    assert titles("zeca") == ["Flac Song"]  # busca também no artista
    assert titles("inexistente") == []
    assert len(titles("")) == 3


def test_list_is_sorted_by_title(client, samples):
    for key in ("mp3", "wav", "flac"):
        import_(client, samples[key])
    titles = [s["title"] for s in client.get("/songs", headers=H).json()]
    assert titles == sorted(titles, key=str.casefold)


def test_get_and_delete(client, samples):
    song = import_(client, samples["mp3"]).json()
    assert client.get(f"/songs/{song['id']}", headers=H).json()["title"] == song["title"]
    songs_dir = client.app.state.settings.data_dir / "songs"
    assert any(songs_dir.iterdir())

    assert client.delete(f"/songs/{song['id']}", headers=H).status_code == 204
    assert client.get(f"/songs/{song['id']}", headers=H).status_code == 404
    assert client.delete(f"/songs/{song['id']}", headers=H).status_code == 404
    assert not any(songs_dir.iterdir())
    # depois de remover, a mesma música pode ser importada de novo
    assert import_(client, samples["mp3"]).status_code == 201


def test_missing_ffprobe_gives_a_clear_error(client, samples, monkeypatch):
    monkeypatch.setenv("KARAOKE_FFPROBE", "ffprobe-que-nao-existe")
    r = import_(client, samples["mp3"])
    assert r.status_code == 503 and r.json()["code"] == "tool_unavailable"


def test_many_concurrent_requests_do_not_break_the_connection(client, samples):
    """A conexão SQLite é aberta e usada em threads diferentes do pool do FastAPI."""
    from concurrent.futures import ThreadPoolExecutor

    import_(client, samples["mp3"])

    def call(_: int) -> int:
        return client.get("/songs", headers=H).status_code

    with ThreadPoolExecutor(max_workers=16) as pool:
        codes = list(pool.map(call, range(200)))
    assert set(codes) == {200}
