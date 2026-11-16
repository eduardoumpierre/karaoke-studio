from __future__ import annotations

import logging
import threading
import time
from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware

from . import __version__
from .api import analysis, health, jobs, lyrics, pitch, songs
from .config import ALLOWED_ORIGINS, TOKEN_HEADER, Settings
from .db import migrate
from .security import require_token
from .services.analyzer import Analyzer, LibrosaAnalyzer
from .services.jobs import JobRunner
from .services.library import LibraryError
from .services.lrclib import LrclibClient
from .services.separator import DemucsSeparator, Separator

http_log = logging.getLogger("karaoke.http")
# Rotas consultadas em loop pela interface: não vale a pena registrá-las.
_QUIET_PREFIXES = ("/health", "/jobs")


def create_app(
    settings: Settings,
    separator: Separator | None = None,
    lrclib: LrclibClient | None = None,
    analyzer: Analyzer | None = None,
) -> FastAPI:
    analyzer = analyzer or LibrosaAnalyzer(settings.numba_cache_dir if settings.warm_up else None)
    runner = JobRunner(settings, separator or DemucsSeparator(settings.data_dir), analyzer)

    @asynccontextmanager
    async def lifespan(_: FastAPI):
        settings.data_dir.mkdir(parents=True, exist_ok=True)
        migrate(settings.db_path)
        runner.start()
        warm = getattr(analyzer, "warm_up", None)
        if settings.warm_up and warm:
            threading.Thread(target=warm, name="analysis-warmup", daemon=True).start()
        try:
            yield
        finally:
            runner.stop()
            close = getattr(analyzer, "close", None)
            if close:
                close()

    app = FastAPI(title="Karaoke Studio music-engine", version=__version__, lifespan=lifespan)
    app.state.settings = settings
    app.state.runner = runner
    app.state.lrclib = lrclib or LrclibClient()
    app.state.analyzer = analyzer
    app.state.started_at = time.monotonic()
    app.add_middleware(
        CORSMiddleware,
        allow_origins=ALLOWED_ORIGINS,
        allow_methods=["*"],
        allow_headers=[TOKEN_HEADER, "Content-Type"],
    )

    @app.middleware("http")
    async def log_requests(request: Request, call_next):
        path = request.url.path
        if request.method == "OPTIONS" or path.startswith(_QUIET_PREFIXES):
            return await call_next(request)
        started = time.monotonic()
        http_log.info("-> %s %s", request.method, path)
        status = "erro"
        try:
            response = await call_next(request)
            status = str(response.status_code)
            return response
        finally:
            ms = (time.monotonic() - started) * 1000
            http_log.info("<- %s %s %s %.0f ms", request.method, path, status, ms)

    app.add_exception_handler(LibraryError, songs.library_error_handler)  # type: ignore[arg-type]
    # Todas as rotas exigem o token.
    app.include_router(health.router, dependencies=[Depends(require_token)])
    app.include_router(songs.router, dependencies=[Depends(require_token)])
    app.include_router(jobs.router, dependencies=[Depends(require_token)])
    app.include_router(lyrics.router, dependencies=[Depends(require_token)])
    app.include_router(analysis.router, dependencies=[Depends(require_token)])
    app.include_router(pitch.router, dependencies=[Depends(require_token)])
    return app
