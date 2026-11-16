"""Chamadas ao ffmpeg que podem ser canceladas: normalização e codificação FLAC."""

from __future__ import annotations

import os
import subprocess
import tempfile
import threading
import time
from pathlib import Path

# Sem janela de console extra no Windows.
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


class Cancelled(Exception):
    """O job foi cancelado pelo usuário."""


class AudioToolError(Exception):
    """Uma ferramenta de áudio (ffmpeg, Demucs) falhou."""


def ffmpeg_command() -> str:
    return os.environ.get("KARAOKE_FFMPEG", "ffmpeg")


def stop_process(proc: subprocess.Popen) -> None:
    proc.kill()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:  # pragma: no cover
        pass


def run_cancellable(cmd: list[str], cancel: threading.Event, what: str) -> None:
    """Roda o comando até o fim; se `cancel` for ativado, mata o processo e levanta Cancelled."""
    with tempfile.TemporaryFile() as err:
        try:
            proc = subprocess.Popen(
                cmd, stdout=subprocess.DEVNULL, stderr=err, creationflags=NO_WINDOW
            )
        except FileNotFoundError as exc:
            raise AudioToolError(f"{what}: programa não encontrado ({cmd[0]}).") from exc
        while proc.poll() is None:
            if cancel.is_set():
                stop_process(proc)
                raise Cancelled
            time.sleep(0.05)
        if proc.returncode != 0:
            err.seek(0)
            tail = err.read().decode("utf-8", "replace").strip().splitlines()[-3:]
            raise AudioToolError(f"{what} falhou: {' | '.join(tail) or proc.returncode}")


def normalize_to_wav(src: Path, dst: Path, cancel: threading.Event) -> None:
    """Qualquer áudio vira WAV estéreo de 44,1 kHz (float 32 bits), o formato do pipeline."""
    cmd = [ffmpeg_command(), "-y", "-loglevel", "error", "-i", str(src)]
    cmd += ["-vn", "-ar", "44100", "-ac", "2", "-c:a", "pcm_f32le", str(dst)]
    run_cancellable(cmd, cancel, "Normalização do áudio")


def encode_flac(src: Path, dst: Path, cancel: threading.Event) -> None:
    """FLAC sem perdas em 24 bits (o FLAC não guarda float)."""
    cmd = [ffmpeg_command(), "-y", "-loglevel", "error", "-i", str(src)]
    cmd += ["-c:a", "flac", "-sample_fmt", "s32", "-compression_level", "5", str(dst)]
    run_cancellable(cmd, cancel, "Codificação FLAC")
