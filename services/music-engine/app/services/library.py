"""Biblioteca de músicas: importar (validar, deduplicar, copiar), listar, buscar e remover."""

from __future__ import annotations

import hashlib
import shutil
import sqlite3
import unicodedata
from pathlib import Path

from ..config import Settings
from ..models.song import AudioFiles, Song
from .probe import ProbeError, ToolMissing, probe

ALLOWED_EXTENSIONS = {".mp3", ".wav", ".flac"}

_SONG_COLUMNS = (
    "id, title, artist, album, duration_seconds, format, codec, sample_rate, channels, "
    "bit_rate, size_bytes, original_name, status, created_at"
)


class LibraryError(Exception):
    status_code = 400
    code = "library_error"

    def __init__(self, message: str, song_id: int | None = None) -> None:
        super().__init__(message)
        self.message = message
        self.song_id = song_id


class FileMissing(LibraryError):
    status_code = 404
    code = "file_not_found"


class UnsupportedFormat(LibraryError):
    status_code = 422
    code = "unsupported_format"


class InvalidAudio(LibraryError):
    status_code = 400
    code = "invalid_audio"


class DuplicateSong(LibraryError):
    status_code = 409
    code = "duplicate"


class ToolUnavailable(LibraryError):
    status_code = 503
    code = "tool_unavailable"


class SongBusy(LibraryError):
    status_code = 409
    code = "song_busy"


class SongNotFound(LibraryError):
    status_code = 404
    code = "song_not_found"


def _fold(text: str) -> str:
    """Minúsculas e sem acentos, para a busca."""
    decomposed = unicodedata.normalize("NFKD", text.casefold())
    return "".join(c for c in decomposed if not unicodedata.combining(c))


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


class Library:
    def __init__(self, settings: Settings, conn: sqlite3.Connection) -> None:
        self.settings = settings
        self.conn = conn

    # ---------------------------------------------------------------- importar
    def import_file(self, source: Path) -> Song:
        if not source.is_file():
            raise FileMissing(f"Arquivo não encontrado: {source}")
        ext = source.suffix.lower()
        if ext not in ALLOWED_EXTENSIONS:
            allowed = ", ".join(sorted(e.lstrip(".") for e in ALLOWED_EXTENSIONS))
            raise UnsupportedFormat(
                f"Formato não suportado ({ext or 'sem extensão'}). Use: {allowed}."
            )
        try:
            info = probe(source)
        except ProbeError as exc:
            raise InvalidAudio(str(exc)) from exc
        except ToolMissing as exc:
            raise ToolUnavailable(str(exc)) from exc

        file_hash = _sha256(source)
        existing = self.conn.execute(
            "SELECT id, title FROM songs WHERE file_hash = ?", (file_hash,)
        ).fetchone()
        if existing:
            raise DuplicateSong(
                f"Esta música já está na biblioteca: {existing['title']}.", song_id=existing["id"]
            )

        rel_dir = Path("songs") / file_hash[:16]
        dest_dir = self.settings.data_dir / rel_dir
        dest = dest_dir / f"original{ext}"
        dest_dir.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_suffix(dest.suffix + ".part")
        try:
            shutil.copyfile(source, tmp)
            tmp.replace(dest)
            cur = self.conn.execute(
                "INSERT INTO songs (title, artist, album, duration_seconds, format, codec, "
                "sample_rate, channels, bit_rate, size_bytes, file_hash, original_name, "
                "stored_path) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    info.title,
                    info.artist,
                    info.album,
                    info.duration_seconds,
                    ext.lstrip("."),
                    info.codec,
                    info.sample_rate,
                    info.channels,
                    info.bit_rate,
                    dest.stat().st_size,
                    file_hash,
                    source.name,
                    (rel_dir / dest.name).as_posix(),
                ),
            )
            self.conn.commit()
        except Exception:
            self.conn.rollback()
            shutil.rmtree(dest_dir, ignore_errors=True)
            raise
        return self.get(cur.lastrowid)  # type: ignore[arg-type]

    # ---------------------------------------------------------------- consultar
    def get(self, song_id: int) -> Song:
        row = self.conn.execute(
            f"SELECT {_SONG_COLUMNS} FROM songs WHERE id = ?", (song_id,)
        ).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        return Song(**dict(row))

    def audio_files(self, song_id: int) -> AudioFiles:
        row = self.conn.execute("SELECT stored_path FROM songs WHERE id = ?", (song_id,)).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        original = self.settings.data_dir / row["stored_path"]
        if not original.is_file():
            raise FileMissing("O arquivo da música não está mais na pasta de dados.", song_id)
        stems = original.parent / "stems"
        vocals, instrumental = stems / "vocals.flac", stems / "instrumental.flac"
        has_stems = vocals.is_file() and instrumental.is_file()
        return AudioFiles(
            original=str(original),
            vocals=str(vocals) if has_stems else None,
            instrumental=str(instrumental) if has_stems else None,
        )

    def list(self, query: str = "") -> list[Song]:
        rows = self.conn.execute(
            f"SELECT {_SONG_COLUMNS} FROM songs ORDER BY title COLLATE NOCASE, id"
        ).fetchall()
        songs = [Song(**dict(r)) for r in rows]
        needle = _fold(query.strip())
        if not needle:
            return songs
        return [s for s in songs if needle in _fold(f"{s.title} {s.artist} {s.album}")]

    # ---------------------------------------------------------------- remover
    def delete(self, song_id: int) -> None:
        row = self.conn.execute("SELECT stored_path FROM songs WHERE id = ?", (song_id,)).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        busy = self.conn.execute(
            "SELECT 1 FROM jobs WHERE song_id = ? AND status IN ('queued', 'running')", (song_id,)
        ).fetchone()
        if busy:
            raise SongBusy(
                "Esta música está sendo processada. Cancele o job antes de remover.", song_id
            )
        song_dir = (self.settings.data_dir / row["stored_path"]).parent
        self.conn.execute("DELETE FROM songs WHERE id = ?", (song_id,))
        self.conn.commit()
        songs_root = (self.settings.data_dir / "songs").resolve()
        if song_dir.resolve().is_relative_to(songs_root):
            shutil.rmtree(song_dir, ignore_errors=True)
