from __future__ import annotations

from pydantic import BaseModel


class Song(BaseModel):
    id: int
    title: str
    artist: str
    album: str
    duration_seconds: float
    format: str
    codec: str
    sample_rate: int
    channels: int
    bit_rate: int | None
    size_bytes: int
    original_name: str
    status: str
    created_at: str


class ImportRequest(BaseModel):
    path: str


class ApiError(BaseModel):
    code: str
    message: str
    song_id: int | None = None


class AudioFiles(BaseModel):
    """Caminhos absolutos dos arquivos de áudio da música (para o player do Tauri)."""

    original: str
    vocals: str | None
    instrumental: str | None
