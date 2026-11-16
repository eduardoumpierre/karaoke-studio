"""Letras: busca no LRCLIB, cache no SQLite, letra colada à mão e ajuste de tempo."""

from __future__ import annotations

import sqlite3

from ..models.lyrics import LyricLine, Lyrics
from .library import LibraryError, SongNotFound
from .lrc import looks_synced, parse_lrc, strip_to_plain
from .lrclib import LrclibClient, OfflineError, clean_title


class LyricsOffline(LibraryError):
    status_code = 503
    code = "offline"


class LyricsInvalid(LibraryError):
    status_code = 422
    code = "invalid_lyrics"


class NoLyrics(LibraryError):
    status_code = 404
    code = "no_lyrics"


class LyricsService:
    def __init__(self, conn: sqlite3.Connection, client: LrclibClient) -> None:
        self.conn = conn
        self.client = client

    def _song(self, song_id: int) -> sqlite3.Row:
        row = self.conn.execute(
            "SELECT id, title, artist, album, duration_seconds FROM songs WHERE id = ?",
            (song_id,),
        ).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        return row

    def get(self, song_id: int) -> Lyrics:
        self._song(song_id)
        row = self.conn.execute("SELECT * FROM lyrics WHERE song_id = ?", (song_id,)).fetchone()
        if row is None:
            return Lyrics(status="none")
        lines = [
            LyricLine(time_ms=ln.time_ms, text=ln.text) for ln in parse_lrc(row["synced_lrc"] or "")
        ]
        return Lyrics(
            status=row["status"],
            synced=bool(lines),
            lines=lines,
            plain=row["plain"],
            source=row["source"],
            offset_ms=row["offset_ms"],
            matched_title=row["matched_title"],
            matched_artist=row["matched_artist"],
        )

    def _save(
        self,
        song_id: int,
        status: str,
        synced: str | None,
        plain: str | None,
        source: str | None,
        title: str | None = None,
        artist: str | None = None,
    ) -> None:
        self.conn.execute(
            "INSERT INTO lyrics (song_id, status, synced_lrc, plain, source, matched_title, "
            "matched_artist, offset_ms) VALUES (?, ?, ?, ?, ?, ?, ?, 0) "
            "ON CONFLICT(song_id) DO UPDATE SET status=excluded.status, "
            "synced_lrc=excluded.synced_lrc, plain=excluded.plain, source=excluded.source, "
            "matched_title=excluded.matched_title, matched_artist=excluded.matched_artist, "
            "offset_ms=0, fetched_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            (song_id, status, synced, plain, source, title, artist),
        )
        self.conn.commit()

    def fetch(self, song_id: int, title: str | None = None, artist: str | None = None) -> Lyrics:
        song = self._song(song_id)
        if title and title.strip():
            search_title, search_artist = title.strip(), (artist or "").strip()
        else:
            search_title, search_artist = clean_title(song["title"], song["artist"])
            if artist and artist.strip():
                search_artist = artist.strip()
        try:
            found = self.client.find(
                search_title, search_artist, song["album"], song["duration_seconds"]
            )
        except OfflineError as exc:
            # Sem rede: não grava nada, para tentar de novo depois.
            raise LyricsOffline(
                "Não foi possível falar com o LRCLIB (sem internet?). Tente de novo mais tarde.",
                song_id,
            ) from exc
        if found is None:
            self._save(song_id, "not_found", None, None, None)
        else:
            plain = found.plain or (strip_to_plain(found.synced) if found.synced else None)
            synced = found.synced if found.synced and looks_synced(found.synced) else None
            self._save(song_id, "found", synced, plain, "lrclib", found.title, found.artist)
        return self.get(song_id)

    def set_manual(self, song_id: int, text: str) -> Lyrics:
        self._song(song_id)
        text = text.strip()
        if not text:
            raise LyricsInvalid("A letra está vazia.", song_id)
        synced = text if looks_synced(text) else None
        plain = strip_to_plain(text)
        if not plain:
            raise LyricsInvalid("A letra está vazia.", song_id)
        self._save(song_id, "found", synced, plain, "manual")
        return self.get(song_id)

    def set_offset(self, song_id: int, offset_ms: int) -> Lyrics:
        self._song(song_id)
        cur = self.conn.execute(
            "UPDATE lyrics SET offset_ms = ? WHERE song_id = ?", (offset_ms, song_id)
        )
        self.conn.commit()
        if cur.rowcount == 0:
            raise NoLyrics("Esta música ainda não tem letra.", song_id)
        return self.get(song_id)

    def delete(self, song_id: int) -> None:
        self._song(song_id)
        self.conn.execute("DELETE FROM lyrics WHERE song_id = ?", (song_id,))
        self.conn.commit()
