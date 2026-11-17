from __future__ import annotations

from pydantic import BaseModel


class Job(BaseModel):
    id: int
    song_id: int
    kind: str
    status: str  # queued, running, done, failed, cancelled
    stage: str | None
    progress: float  # 0 a 1
    error: str | None
    created_at: str
    updated_at: str
    song_title: str | None = None
