from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field


class LyricLine(BaseModel):
    time_ms: int
    text: str


class Lyrics(BaseModel):
    # none = nunca buscada; not_found = buscada e o LRCLIB não tem; found = tem letra
    status: Literal["none", "found", "not_found"]
    synced: bool = False
    lines: list[LyricLine] = []
    plain: str | None = None
    source: str | None = None
    offset_ms: int = 0
    matched_title: str | None = None
    matched_artist: str | None = None


class FetchLyricsRequest(BaseModel):
    """Título e artista para a busca; se vazios, usa os da música (já limpos)."""

    title: str | None = Field(default=None, max_length=300)
    artist: str | None = Field(default=None, max_length=300)


class ManualLyricsRequest(BaseModel):
    text: str = Field(min_length=1, max_length=200_000)


class OffsetRequest(BaseModel):
    offset_ms: int = Field(ge=-30_000, le=30_000)
