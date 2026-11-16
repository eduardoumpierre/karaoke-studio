from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Request

from ..models.pitch import Pitch, PitchRequest, PitchResult
from ..models.song import ApiError
from ..services.pitch import PitchService
from .jobs import DbDep

router = APIRouter(tags=["pitch"])

_ERRORS = {
    404: {"model": ApiError},
    409: {"model": ApiError},
    422: {"model": ApiError},
    500: {"model": ApiError},
}


def get_pitch_service(request: Request, conn: DbDep) -> PitchService:
    return PitchService(conn, request.app.state.settings.data_dir)


PitchDep = Annotated[PitchService, Depends(get_pitch_service)]


@router.get(
    "/songs/{song_id}/pitch", response_model=Pitch, responses=_ERRORS, operation_id="getPitch"
)
def get_pitch(song_id: int, service: PitchDep) -> Pitch:
    return service.get(song_id)


@router.put(
    "/songs/{song_id}/pitch", response_model=PitchResult, responses=_ERRORS, operation_id="setPitch"
)
def set_pitch(song_id: int, body: PitchRequest, service: PitchDep) -> PitchResult:
    """Define o tom da música e devolve os arquivos para tocar (renderiza o que faltar)."""
    return service.apply(song_id, body.semitones)


@router.delete(
    "/songs/{song_id}/pitch/cache",
    response_model=Pitch,
    responses=_ERRORS,
    operation_id="clearPitchCache",
)
def clear_pitch_cache(song_id: int, service: PitchDep) -> Pitch:
    return service.clear_cache(song_id)
