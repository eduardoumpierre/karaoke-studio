from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Request

from ..models.analysis import Analysis, AnalysisEdit
from ..models.song import ApiError
from ..services.analysis import AnalysisService
from ..services.analyzer import Analyzer
from ..services.jobs import JobRunner, JobStore
from .jobs import DbDep

router = APIRouter(tags=["analysis"])

_ERRORS = {404: {"model": ApiError}, 500: {"model": ApiError}}


def get_analysis_service(request: Request, conn: DbDep) -> AnalysisService:
    analyzer: Analyzer = request.app.state.analyzer
    return AnalysisService(conn, request.app.state.settings.data_dir, analyzer)


AnalysisDep = Annotated[AnalysisService, Depends(get_analysis_service)]


@router.get(
    "/songs/{song_id}/analysis",
    response_model=Analysis,
    responses=_ERRORS,
    operation_id="getAnalysis",
)
def get_analysis(song_id: int, service: AnalysisDep) -> Analysis:
    return service.get(song_id)


@router.post(
    "/songs/{song_id}/analysis/run",
    response_model=Analysis,
    status_code=202,
    responses=_ERRORS,
    operation_id="runAnalysis",
)
def run_analysis(
    song_id: int, service: AnalysisDep, conn: DbDep, request: Request, force: bool = False
) -> Analysis:
    """Põe a análise na fila de tarefas e responde na hora; o andamento vem do GET."""
    service.get(song_id)  # 404 se a música não existe
    store = JobStore(conn)
    if store.active_for_song(song_id, "analyze") is None:  # idempotente
        store.create(song_id, "analyze", force)
        runner: JobRunner = request.app.state.runner
        runner.notify()
    return service.get(song_id)


@router.put(
    "/songs/{song_id}/analysis",
    response_model=Analysis,
    responses=_ERRORS,
    operation_id="editAnalysis",
)
def edit_analysis(song_id: int, body: AnalysisEdit, service: AnalysisDep) -> Analysis:
    return service.edit(song_id, body)
