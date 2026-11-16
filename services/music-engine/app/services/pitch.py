"""Transposição por pré-render: gera os stems no tom pedido com o Rubber Band (via ffmpeg).

Os arquivos ficam em songs/<hash>/pitch/<+N>/ e servem de cache. Trocar de tom é só tocar
outro arquivo, então o motor de áudio não faz processamento pesado em tempo real.
"""

from __future__ import annotations

import os
import shutil
import sqlite3
import threading
from pathlib import Path

from ..models.pitch import MAX_SHIFT, Pitch, PitchResult
from ..models.song import AudioFiles
from .audio_tools import AudioToolError, Cancelled, ffmpeg_command, run_cancellable
from .library import FileMissing, LibraryError, SongBusy, SongNotFound

# Um render por vez: são tarefas de CPU e o app é de uso pessoal.
_RENDER_LOCK = threading.Lock()

# Pedido de tom em andamento por música. Um pedido novo cancela o anterior: quem clica ♯ várias
# vezes só quer o último tom, e renderizar os intermediários prenderia o motor por minutos.
_ACTIVE: dict[int, threading.Event] = {}
_ACTIVE_LOCK = threading.Lock()


class PitchFailed(LibraryError):
    status_code = 500
    code = "pitch_failed"


class PitchSuperseded(LibraryError):
    status_code = 409
    code = "pitch_superseded"


def pitch_filter(semitones: int, voice: bool) -> str:
    ratio = 2 ** (semitones / 12)
    opts = f"pitch={ratio:.6f}:pitchq=quality:transients=smooth"
    if voice:
        opts += ":formant=preserved"  # mantém o timbre da voz em vez de virar "esquilo"
    # Limitador leve: o deslocamento pode criar picos acima de 0 dBFS.
    return f"rubberband={opts},alimiter=limit=0.97"


def render_shifted(
    src: Path, dst: Path, semitones: int, voice: bool, cancel: threading.Event | None = None
) -> None:
    tmp = dst.with_name(f"{dst.stem}.part.flac")
    cmd = [ffmpeg_command(), "-y", "-loglevel", "error", "-i", str(src)]
    cmd += ["-af", pitch_filter(semitones, voice)]
    cmd += ["-c:a", "flac", "-sample_fmt", "s32", "-compression_level", "5", str(tmp)]
    try:
        run_cancellable(cmd, cancel or threading.Event(), "Transposição")
        os.replace(tmp, dst)
    finally:
        tmp.unlink(missing_ok=True)


def _fresh(dst: Path, src: Path) -> bool:
    """Pronto e mais novo que a origem (se a música foi separada de novo, refaz)."""
    return dst.is_file() and dst.stat().st_mtime >= src.stat().st_mtime


class PitchService:
    def __init__(self, conn: sqlite3.Connection, data_dir: Path) -> None:
        self.conn = conn
        self.data_dir = data_dir

    def _original(self, song_id: int) -> Path:
        row = self.conn.execute("SELECT stored_path FROM songs WHERE id = ?", (song_id,)).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        path = self.data_dir / row["stored_path"]
        if not path.is_file():
            raise FileMissing("O arquivo da música não está mais na pasta de dados.", song_id)
        return path

    def _current(self, song_id: int) -> int:
        row = self.conn.execute(
            "SELECT semitones FROM song_pitch WHERE song_id = ?", (song_id,)
        ).fetchone()
        return int(row["semitones"]) if row else 0

    def get(self, song_id: int) -> Pitch:
        original = self._original(song_id)
        root = original.parent / "pitch"
        rendered: list[int] = []
        size = 0
        if root.is_dir():
            for d in root.iterdir():
                try:
                    n = int(d.name)
                except ValueError:
                    continue
                if any(d.glob("*.flac")):
                    rendered.append(n)
                size += sum(f.stat().st_size for f in d.glob("*") if f.is_file())
        return Pitch(semitones=self._current(song_id), rendered=sorted(rendered), cache_bytes=size)

    def audio(
        self, song_id: int, semitones: int, cancel: threading.Event | None = None
    ) -> AudioFiles:
        """Arquivos para tocar no tom pedido, renderizando o que faltar."""
        original = self._original(song_id)
        stems = original.parent / "stems"
        vocals, instrumental = stems / "vocals.flac", stems / "instrumental.flac"
        has_stems = vocals.is_file() and instrumental.is_file()
        if semitones == 0:
            return AudioFiles(
                original=str(original),
                vocals=str(vocals) if has_stems else None,
                instrumental=str(instrumental) if has_stems else None,
            )
        out = original.parent / "pitch" / f"{semitones:+d}"
        jobs = (
            [("vocals", vocals, True), ("instrumental", instrumental, False)]
            if has_stems
            else [("original", original, True)]
        )
        cancel = cancel or threading.Event()
        try:
            while not _RENDER_LOCK.acquire(timeout=0.1):
                if cancel.is_set():
                    raise Cancelled
            try:
                out.mkdir(parents=True, exist_ok=True)
                for name, src, voice in jobs:
                    dst = out / f"{name}.flac"
                    if cancel.is_set():
                        raise Cancelled
                    if not _fresh(dst, src):
                        render_shifted(src, dst, semitones, voice, cancel)
            finally:
                _RENDER_LOCK.release()
        except Cancelled as exc:
            raise PitchSuperseded("Um pedido de tom mais novo substituiu este.", song_id) from exc
        except AudioToolError as exc:
            raise PitchFailed(str(exc), song_id) from exc
        if has_stems:
            return AudioFiles(
                original=str(original),
                vocals=str(out / "vocals.flac"),
                instrumental=str(out / "instrumental.flac"),
            )
        return AudioFiles(original=str(out / "original.flac"), vocals=None, instrumental=None)

    def apply(self, song_id: int, semitones: int) -> PitchResult:
        if abs(semitones) > MAX_SHIFT:
            raise PitchFailed(
                f"A transposição vai de −{MAX_SHIFT} a +{MAX_SHIFT} semitons.", song_id
            )
        busy = self.conn.execute(
            "SELECT 1 FROM jobs WHERE song_id = ? AND kind = 'separate' "
            "AND status IN ('queued', 'running')",
            (song_id,),
        ).fetchone()
        if busy:
            raise SongBusy("Esta música está sendo separada. Espere terminar.", song_id)
        mine = threading.Event()
        with _ACTIVE_LOCK:
            previous = _ACTIVE.get(song_id)
            if previous is not None:
                previous.set()
            _ACTIVE[song_id] = mine
        try:
            audio = self.audio(song_id, semitones, mine)
        finally:
            with _ACTIVE_LOCK:
                if _ACTIVE.get(song_id) is mine:
                    del _ACTIVE[song_id]
        self.conn.execute(
            "INSERT INTO song_pitch (song_id, semitones) VALUES (?, ?) "
            "ON CONFLICT(song_id) DO UPDATE SET semitones = excluded.semitones",
            (song_id, semitones),
        )
        self.conn.commit()
        return PitchResult(pitch=self.get(song_id), audio=audio)

    def clear_cache(self, song_id: int) -> Pitch:
        original = self._original(song_id)
        root = original.parent / "pitch"
        if root.resolve().parent == original.parent.resolve():
            shutil.rmtree(root, ignore_errors=True)
        return self.get(song_id)
