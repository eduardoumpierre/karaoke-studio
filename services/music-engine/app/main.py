from __future__ import annotations

import time
from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI
from fastapi.middleware.cors import CORSMiddleware

from . import __version__
from .api import health
from .config import ALLOWED_ORIGINS, TOKEN_HEADER, Settings
from .security import require_token


def create_app(settings: Settings) -> FastAPI:
    @asynccontextmanager
    async def lifespan(_: FastAPI):
        settings.data_dir.mkdir(parents=True, exist_ok=True)
        yield

    app = FastAPI(title="Karaoke Studio music-engine", version=__version__, lifespan=lifespan)
    app.state.settings = settings
    app.state.started_at = time.monotonic()
    app.add_middleware(
        CORSMiddleware,
        allow_origins=ALLOWED_ORIGINS,
        allow_methods=["*"],
        allow_headers=[TOKEN_HEADER, "Content-Type"],
    )
    # Todas as rotas exigem o token.
    app.include_router(health.router, dependencies=[Depends(require_token)])
    return app
