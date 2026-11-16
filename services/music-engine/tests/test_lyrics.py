import json

import pytest
from fastapi.testclient import TestClient

from app.config import TOKEN_HEADER, Settings
from app.main import create_app
from app.services.lrc import looks_synced, parse_lrc, strip_to_plain
from app.services.lrclib import (
    Found,
    LrclibClient,
    OfflineError,
    clean_title,
    pick_best,
)

from .conftest import TOKEN
from .test_library import import_, samples  # noqa: F401  (fixture)

H = {TOKEN_HEADER: TOKEN}

LRC = """[ar:Banda]
[ti:Canção]
[offset:0]
[00:12.00] Primeira linha
[00:15.5]Segunda linha
[01:02.250]Terceira <00:62.50>linha
[00:20.00][01:30.00]Refrão
"""


# ----------------------------------------------------------------------------- parser
def test_parse_lrc_basic():
    lines = parse_lrc(LRC)
    assert [(ln.time_ms, ln.text) for ln in lines] == [
        (12_000, "Primeira linha"),
        (15_500, "Segunda linha"),
        (20_000, "Refrão"),
        (62_250, "Terceira linha"),
        (90_000, "Refrão"),
    ]


def test_parse_lrc_fraction_forms_and_offset_tag():
    assert parse_lrc("[00:01.5]a")[0].time_ms == 1500
    assert parse_lrc("[00:01.50]a")[0].time_ms == 1500
    assert parse_lrc("[00:01.500]a")[0].time_ms == 1500
    assert parse_lrc("[00:01]a")[0].time_ms == 1000
    assert parse_lrc("[00:01:25]a")[0].time_ms == 1250
    # [offset:+500] = a letra deve aparecer 500 ms antes
    assert parse_lrc("[offset:+500]\n[00:10.00]a")[0].time_ms == 9500
    assert parse_lrc("[offset:+99999]\n[00:10.00]a")[0].time_ms == 0


def test_parse_lrc_ignores_junk_and_keeps_empty_lines_as_gaps():
    lines = parse_lrc("texto solto\n[xx:yy]nada\n[00:05.00]\n[00:07.00]ok\n")
    assert [(ln.time_ms, ln.text) for ln in lines] == [(5000, ""), (7000, "ok")]
    assert parse_lrc("") == [] and parse_lrc("só texto\nsem tempos") == []


def test_looks_synced_and_plain():
    assert looks_synced(LRC) and not looks_synced("só texto\nsem tempos")
    assert not looks_synced("[00:01.00] só uma linha")
    plain = strip_to_plain(LRC)
    assert "[" not in plain and "Primeira linha" in plain and "Banda" not in plain
    assert "<" not in plain


# ----------------------------------------------------------------------------- limpeza
@pytest.mark.parametrize(
    ("title", "artist", "expected"),
    [
        (
            "Michael Jackson - The Girl Is Mine (Audio)",
            "Michael Jackson",
            ("The Girl Is Mine", "Michael Jackson"),
        ),
        (
            "My Chemical Romance - Welcome To The Black Parade (Official Music Video)",
            "My Chemical Romance",
            ("Welcome To The Black Parade", "My Chemical Romance"),
        ),
        ("Queen - Bohemian Rhapsody [Remastered 2011]", "", ("Bohemian Rhapsody", "Queen")),
        ("Bohemian Rhapsody", "Queen - Topic", ("Bohemian Rhapsody", "Queen")),
        ("Música (Ao Vivo) | Canal Qualquer", "Fulano", ("Música", "Fulano")),
        ("Título (feat. Fulano)", "Beltrano", ("Título (feat. Fulano)", "Beltrano")),
        ("Nome - com hífen", "Outro Artista", ("Nome - com hífen", "Outro Artista")),
    ],
)
def test_clean_title(title, artist, expected):
    assert clean_title(title, artist) == expected


# ----------------------------------------------------------------------------- escolha do melhor
def item(synced=True, dur=200.0, title="T", artist="A", plain="letra", **extra):
    d = {
        "trackName": title,
        "artistName": artist,
        "duration": dur,
        "plainLyrics": plain,
        "syncedLyrics": "[00:01.00] a\n[00:02.00] b" if synced else None,
    }
    return d | extra


def test_pick_best_prefers_synced_and_close_duration():
    best = pick_best(
        [item(synced=False, dur=200), item(dur=201.5, title="perto"), item(dur=230)], 200
    )
    assert best is not None and best.title == "perto"
    assert pick_best([item(dur=240)], 200) is None  # longe demais: outra versão da música
    assert pick_best([item(instrumental=True)], 200) is None
    assert pick_best([item(synced=False, plain=None)], 200) is None
    plain_only = pick_best([item(synced=False, dur=199)], 200)
    assert plain_only is not None and plain_only.synced is None


# ----------------------------------------------------------------------------- cliente
class FakeHttp:
    def __init__(self, routes):
        self.routes = routes
        self.calls: list[str] = []

    def __call__(self, url):
        self.calls.append(url)
        for key, (status, body) in self.routes.items():
            if key in url:
                return status, json.dumps(body).encode()
        return 404, b"{}"


def test_client_exact_match_stops_early():
    http = FakeHttp({"/api/get?": (200, item(dur=200))})
    found = LrclibClient(http).find("Título", "Artista", "Álbum", 200.2)
    assert found is not None and found.synced
    assert len(http.calls) == 1
    assert "duration=200" in http.calls[0] and "track_name=T%C3%ADtulo" in http.calls[0]


def test_client_falls_back_to_search_then_free_text():
    http = FakeHttp({"/api/search?track_name": (200, [item(dur=199, title="achada")])})
    found = LrclibClient(http).find("T", "A", "", 200)
    assert found is not None and found.title == "achada"
    assert [c.split("?")[0].rsplit("/", 1)[1] for c in http.calls] == ["get", "search"]

    http = FakeHttp({"search?q=": (200, [item(dur=200, title="livre")])})
    found = LrclibClient(http).find("T", "A", "", 200)
    assert found is not None and found.title == "livre"
    assert LrclibClient(FakeHttp({})).find("T", "A", "", 200) is None


def test_client_errors_become_offline():
    def boom(url):
        raise OfflineError("sem rede")

    with pytest.raises(OfflineError):
        LrclibClient(boom).find("T", "A", "", 200)
    with pytest.raises(OfflineError):
        LrclibClient(lambda u: (500, b"erro")).find("T", "A", "", 200)
    with pytest.raises(OfflineError):
        LrclibClient(lambda u: (200, b"<html>")).find("T", "A", "", 200)


# ----------------------------------------------------------------------------- API
class StubClient(LrclibClient):
    def __init__(self, result=None, error=False):
        self.result = result
        self.error = error
        self.calls: list[tuple] = []

    def find(self, title, artist, album, duration):
        self.calls.append((title, artist))
        if self.error:
            raise OfflineError("sem rede")
        return self.result


SYNCED = "[00:01.00] linha um\n[00:03.00] linha dois\n[00:05.00]"


def make_client(tmp_path, stub):
    return TestClient(create_app(Settings(token=TOKEN, data_dir=tmp_path), lrclib=stub))


def song(c, samples):  # noqa: F811
    r = import_(c, samples["mp3"])
    assert r.status_code == 201, r.text
    return r.json()["id"]


def test_requires_token_and_unknown_song(tmp_path):
    with make_client(tmp_path, StubClient()) as c:
        assert c.get("/songs/1/lyrics").status_code == 401
        assert c.get("/songs/999/lyrics", headers=H).status_code == 404
        assert c.post("/songs/999/lyrics/fetch", headers=H).status_code == 404


def test_fetch_found_synced_and_cached(tmp_path, samples):  # noqa: F811
    stub = StubClient(Found(SYNCED, "linha um\nlinha dois", "Canção Número Um", "Banda Teste", 2.0))
    with make_client(tmp_path, stub) as c:
        sid = song(c, samples)
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["status"] == "none"
        r = c.post(f"/songs/{sid}/lyrics/fetch", headers=H)
        assert r.status_code == 200, r.text
        body = r.json()
        assert body["status"] == "found" and body["synced"] and body["source"] == "lrclib"
        assert [ln["time_ms"] for ln in body["lines"]] == [1000, 3000, 5000]
        assert body["matched_artist"] == "Banda Teste"
        assert stub.calls == [("Canção Número Um", "Banda Teste")]
        # persistiu
        again = c.get(f"/songs/{sid}/lyrics", headers=H).json()
        assert again["status"] == "found" and len(again["lines"]) == 3


def test_fetch_uses_overrides(tmp_path, samples):  # noqa: F811
    stub = StubClient(None)
    with make_client(tmp_path, stub) as c:
        sid = song(c, samples)
        r = c.post(
            f"/songs/{sid}/lyrics/fetch", json={"title": "Outro Nome", "artist": "Outro"}, headers=H
        )
        assert r.status_code == 200
        assert stub.calls == [("Outro Nome", "Outro")]


def test_fetch_not_found_is_cached_but_offline_is_not(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, StubClient(None)) as c:
        sid = song(c, samples)
        r = c.post(f"/songs/{sid}/lyrics/fetch", headers=H)
        assert r.json()["status"] == "not_found"
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["status"] == "not_found"
    with make_client(tmp_path / "b", StubClient(error=True)) as c:
        sid = song(c, samples)
        r = c.post(f"/songs/{sid}/lyrics/fetch", headers=H)
        assert r.status_code == 503 and r.json()["code"] == "offline"
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["status"] == "none"


def test_plain_only_result(tmp_path, samples):  # noqa: F811
    stub = StubClient(Found(None, "verso um\nverso dois", "T", "A", 2.0))
    with make_client(tmp_path, stub) as c:
        sid = song(c, samples)
        body = c.post(f"/songs/{sid}/lyrics/fetch", headers=H).json()
        assert body["status"] == "found" and not body["synced"] and body["lines"] == []
        assert body["plain"] == "verso um\nverso dois"


def test_manual_lyrics_synced_and_plain(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, StubClient()) as c:
        sid = song(c, samples)
        r = c.put(f"/songs/{sid}/lyrics", json={"text": SYNCED}, headers=H)
        assert r.status_code == 200 and r.json()["synced"] and r.json()["source"] == "manual"
        r = c.put(f"/songs/{sid}/lyrics", json={"text": "só texto\nsem tempo"}, headers=H)
        body = r.json()
        assert not body["synced"] and body["plain"] == "só texto\nsem tempo"
        assert c.put(f"/songs/{sid}/lyrics", json={"text": "   "}, headers=H).status_code == 422
        assert (
            c.put(f"/songs/{sid}/lyrics", json={"text": "[00:01.00]"}, headers=H).status_code == 422
        )


def test_offset_is_saved_validated_and_reset_by_new_lyrics(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, StubClient()) as c:
        sid = song(c, samples)
        r = c.put(f"/songs/{sid}/lyrics/offset", json={"offset_ms": 300}, headers=H)
        assert r.status_code == 404 and r.json()["code"] == "no_lyrics"
        c.put(f"/songs/{sid}/lyrics", json={"text": SYNCED}, headers=H)
        r = c.put(f"/songs/{sid}/lyrics/offset", json={"offset_ms": -450}, headers=H)
        assert r.status_code == 200 and r.json()["offset_ms"] == -450
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["offset_ms"] == -450
        assert (
            c.put(f"/songs/{sid}/lyrics/offset", json={"offset_ms": 99999}, headers=H).status_code
            == 422
        )
        c.put(f"/songs/{sid}/lyrics", json={"text": SYNCED}, headers=H)
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["offset_ms"] == 0


def test_delete_lyrics_and_cascade_with_song(tmp_path, samples):  # noqa: F811
    with make_client(tmp_path, StubClient()) as c:
        sid = song(c, samples)
        c.put(f"/songs/{sid}/lyrics", json={"text": SYNCED}, headers=H)
        assert c.delete(f"/songs/{sid}/lyrics", headers=H).status_code == 204
        assert c.get(f"/songs/{sid}/lyrics", headers=H).json()["status"] == "none"
        c.put(f"/songs/{sid}/lyrics", json={"text": SYNCED}, headers=H)
        assert c.delete(f"/songs/{sid}", headers=H).status_code == 204
        import sqlite3

        with sqlite3.connect(tmp_path / "library.db") as db:
            assert db.execute("SELECT count(*) FROM lyrics").fetchone()[0] == 0
