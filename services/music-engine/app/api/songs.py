from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Annotated

from fastapi import APIRouter, Depends, Query, Request, Response
from fastapi.responses import JSONResponse

from ..config import Settings
from ..db import connect
from ..models.song import ApiError, AudioFiles, ImportRequest, Song
from ..security import get_settings
from ..services.library import Library, LibraryError

router = APIRouter(tags=["songs"])

_ERRORS = {
    400: {"model": ApiError},
    404: {"model": ApiError},
    409: {"model": ApiError},
    422: {"model": ApiError},
    503: {"model": ApiError},
}


def get_db(settings: Annotated[Settings, Depends(get_settings)]):
    conn = connect(settings.db_path)
    try:
        yield conn
    finally:
        conn.close()


def get_library(
    settings: Annotated[Settings, Depends(get_settings)],
    conn: Annotated[sqlite3.Connection, Depends(get_db)],
) -> Library:
    return Library(settings, conn)


LibraryDep = Annotated[Library, Depends(get_library)]


async def library_error_handler(_: Request, exc: LibraryError) -> JSONResponse:
    body = ApiError(code=exc.code, message=exc.message, song_id=exc.song_id)
    return JSONResponse(status_code=exc.status_code, content=body.model_dump())


@router.get("/songs", response_model=list[Song], operation_id="listSongs")
def list_songs(library: LibraryDep, q: Annotated[str, Query(max_length=200)] = "") -> list[Song]:
    return library.list(q)


@router.get("/songs/{song_id}", response_model=Song, responses=_ERRORS, operation_id="getSong")
def get_song(song_id: int, library: LibraryDep) -> Song:
    return library.get(song_id)


@router.get(
    "/songs/{song_id}/audio",
    response_model=AudioFiles,
    responses=_ERRORS,
    operation_id="getSongAudio",
)
def get_song_audio(song_id: int, library: LibraryDep) -> AudioFiles:
    return library.audio_files(song_id)


@router.post(
    "/songs/import",
    response_model=Song,
    status_code=201,
    responses=_ERRORS,
    operation_id="importSong",
)
def import_song(body: ImportRequest, library: LibraryDep) -> Song:
    return library.import_file(Path(body.path))


@router.delete("/songs/{song_id}", status_code=204, responses=_ERRORS, operation_id="deleteSong")
def delete_song(song_id: int, library: LibraryDep) -> Response:
    library.delete(song_id)
    return Response(status_code=204)
