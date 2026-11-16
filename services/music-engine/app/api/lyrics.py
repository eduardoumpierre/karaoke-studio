from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Request, Response

from ..models.lyrics import FetchLyricsRequest, Lyrics, ManualLyricsRequest, OffsetRequest
from ..models.song import ApiError
from ..services.lrclib import LrclibClient
from ..services.lyrics import LyricsService
from .jobs import DbDep

router = APIRouter(tags=["lyrics"])

_ERRORS = {
    404: {"model": ApiError},
    422: {"model": ApiError},
    503: {"model": ApiError},
}


def get_lyrics_service(request: Request, conn: DbDep) -> LyricsService:
    client: LrclibClient = request.app.state.lrclib
    return LyricsService(conn, client)


LyricsDep = Annotated[LyricsService, Depends(get_lyrics_service)]


@router.get(
    "/songs/{song_id}/lyrics", response_model=Lyrics, responses=_ERRORS, operation_id="getLyrics"
)
def get_lyrics(song_id: int, service: LyricsDep) -> Lyrics:
    return service.get(song_id)


@router.post(
    "/songs/{song_id}/lyrics/fetch",
    response_model=Lyrics,
    responses=_ERRORS,
    operation_id="fetchLyrics",
)
def fetch_lyrics(
    song_id: int, service: LyricsDep, body: FetchLyricsRequest | None = None
) -> Lyrics:
    body = body or FetchLyricsRequest()
    return service.fetch(song_id, body.title, body.artist)


@router.put(
    "/songs/{song_id}/lyrics", response_model=Lyrics, responses=_ERRORS, operation_id="setLyrics"
)
def set_lyrics(song_id: int, body: ManualLyricsRequest, service: LyricsDep) -> Lyrics:
    return service.set_manual(song_id, body.text)


@router.put(
    "/songs/{song_id}/lyrics/offset",
    response_model=Lyrics,
    responses=_ERRORS,
    operation_id="setLyricsOffset",
)
def set_lyrics_offset(song_id: int, body: OffsetRequest, service: LyricsDep) -> Lyrics:
    return service.set_offset(song_id, body.offset_ms)


@router.delete(
    "/songs/{song_id}/lyrics", status_code=204, responses=_ERRORS, operation_id="deleteLyrics"
)
def delete_lyrics(song_id: int, service: LyricsDep) -> Response:
    service.delete(song_id)
    return Response(status_code=204)
