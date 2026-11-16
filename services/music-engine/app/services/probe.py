"""Leitura de metadados e validação de arquivos de áudio com o ffprobe."""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path


class ProbeError(Exception):
    """O arquivo não é um áudio legível."""


class ToolMissing(Exception):
    """O ffprobe não está instalado."""


@dataclass(frozen=True)
class AudioInfo:
    title: str
    artist: str
    album: str
    duration_seconds: float
    codec: str
    sample_rate: int
    channels: int
    bit_rate: int | None


def ffprobe_command() -> str:
    return os.environ.get("KARAOKE_FFPROBE", "ffprobe")


def _int_or_none(value: object) -> int | None:
    try:
        return int(str(value))
    except (TypeError, ValueError):
        return None


def probe(path: Path) -> AudioInfo:
    try:
        proc = subprocess.run(
            [
                ffprobe_command(),
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                str(path),
            ],
            capture_output=True,
            timeout=60,
            check=False,
        )
    except FileNotFoundError as exc:
        raise ToolMissing(
            "ffprobe não encontrado. Instale o FFmpeg e coloque-o no PATH "
            "(ou defina KARAOKE_FFPROBE)."
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise ProbeError("Tempo esgotado ao ler o arquivo.") from exc
    if proc.returncode != 0:
        raise ProbeError("O arquivo não é um áudio válido ou está corrompido.")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise ProbeError("Resposta inesperada do ffprobe.") from exc

    stream = next((s for s in data.get("streams", []) if s.get("codec_type") == "audio"), None)
    if stream is None:
        raise ProbeError("O arquivo não contém áudio.")
    fmt = data.get("format", {})
    try:
        duration = float(fmt.get("duration") or stream.get("duration") or 0)
    except ValueError:
        duration = 0.0
    if duration <= 0:
        raise ProbeError("Não foi possível determinar a duração do áudio.")
    sample_rate = _int_or_none(stream.get("sample_rate")) or 0
    channels = _int_or_none(stream.get("channels")) or 0
    if sample_rate <= 0 or channels <= 0:
        raise ProbeError("Formato de áudio inválido.")

    tags = {k.lower(): str(v).strip() for k, v in (fmt.get("tags") or {}).items()}
    title = tags.get("title") or path.stem.replace("_", " ").strip() or path.name
    return AudioInfo(
        title=title,
        artist=tags.get("artist", ""),
        album=tags.get("album", ""),
        duration_seconds=round(duration, 3),
        codec=str(stream.get("codec_name", "")),
        sample_rate=sample_rate,
        channels=channels,
        bit_rate=_int_or_none(fmt.get("bit_rate")),
    )
