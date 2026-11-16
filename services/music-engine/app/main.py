from __future__ import annotations

import time
from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI
from fastapi.middleware.cors import CORSMiddleware

from . import __version__
from .api import health, jobs, lyrics, songs
from .config import ALLOWED_ORIGINS, TOKEN_HEADER, Settings
from .db import migrate
from .security import require_token
from .services.jobs import JobRunner
from .services.library import LibraryError
from .services.lrclib import LrclibClient
from .services.separator import DemucsSeparator, Separator


def create_app(
    settings: Settings,
    separator: Separator | None = None,
    lrclib: LrclibClient | None = None,
) -> FastAPI:
    runner = JobRunner(settings, separator or DemucsSeparator(settings.data_dir))

    @asynccontextmanager
    async def lifespan(_: FastAPI):
        settings.data_dir.mkdir(parents=True, exist_ok=True)
        migrate(settings.db_path)
        runner.start()
        try:
            yield
        finally:
            runner.stop()

    app = FastAPI(title="Karaoke Studio music-engine", version=__version__, lifespan=lifespan)
    app.state.settings = settings
    app.state.runner = runner
    app.state.lrclib = lrclib or LrclibClient()
    app.state.started_at = time.monotonic()
    app.add_middleware(
        CORSMiddleware,
        allow_origins=ALLOWED_ORIGINS,
        allow_methods=["*"],
        allow_headers=[TOKEN_HEADER, "Content-Type"],
    )
    app.add_exception_handler(LibraryError, songs.library_error_handler)  # type: ignore[arg-type]
    # Todas as rotas exigem o token.
    app.include_router(health.router, dependencies=[Depends(require_token)])
    app.include_router(songs.router, dependencies=[Depends(require_token)])
    app.include_router(jobs.router, dependencies=[Depends(require_token)])
    app.include_router(lyrics.router, dependencies=[Depends(require_token)])
    return app
