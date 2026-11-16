from __future__ import annotations

import sqlite3
from typing import Annotated

from fastapi import APIRouter, Depends, Query, Request

from ..models.job import Job
from ..models.song import ApiError
from ..services.jobs import (
    ACTIVE,
    AlreadySeparated,
    JobRunner,
    JobStore,
    SeparatorUnavailable,
)
from .songs import LibraryDep, get_db

router = APIRouter(tags=["jobs"])

_ERRORS = {404: {"model": ApiError}, 409: {"model": ApiError}, 503: {"model": ApiError}}


def get_runner(request: Request) -> JobRunner:
    return request.app.state.runner


RunnerDep = Annotated[JobRunner, Depends(get_runner)]
DbDep = Annotated[sqlite3.Connection, Depends(get_db)]


@router.post(
    "/songs/{song_id}/separate",
    response_model=Job,
    status_code=202,
    responses=_ERRORS,
    operation_id="separateSong",
)
def separate_song(
    song_id: int,
    library: LibraryDep,
    runner: RunnerDep,
    conn: DbDep,
    force: Annotated[bool, Query()] = False,
) -> Job:
    song = library.get(song_id)  # 404 se não existir
    store = JobStore(conn)
    existing = store.active_for_song(song_id, "separate")
    if existing:
        return existing  # idempotente: não enfileira duas vezes
    if song.status == "separated" and not force:
        raise AlreadySeparated("Esta música já foi separada.", song_id=song_id)
    reason = runner.separator.unavailable_reason()
    if reason:
        raise SeparatorUnavailable(reason, song_id=song_id)
    job = store.create(song_id, "separate")
    runner.notify()
    return job


@router.get("/jobs", response_model=list[Job], operation_id="listJobs")
def list_jobs(conn: DbDep, active: Annotated[bool, Query()] = False) -> list[Job]:
    return JobStore(conn).list(active_only=active)


@router.get("/jobs/{job_id}", response_model=Job, responses=_ERRORS, operation_id="getJob")
def get_job(job_id: int, conn: DbDep) -> Job:
    return JobStore(conn).get(job_id)


@router.post(
    "/jobs/{job_id}/cancel", response_model=Job, responses=_ERRORS, operation_id="cancelJob"
)
def cancel_job(job_id: int, runner: RunnerDep) -> Job:
    return runner.cancel(job_id)


__all__ = ["ACTIVE", "router"]
