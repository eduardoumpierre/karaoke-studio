from __future__ import annotations

import sys
import time

from fastapi import APIRouter, Request
from pydantic import BaseModel

from .. import __version__

router = APIRouter(tags=["health"])


class Health(BaseModel):
    status: str
    version: str
    python: str
    uptime_seconds: float


@router.get("/health", response_model=Health, operation_id="getHealth")
def health(request: Request) -> Health:
    return Health(
        status="ok",
        version=__version__,
        python=f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}",
        uptime_seconds=round(time.monotonic() - request.app.state.started_at, 3),
    )
