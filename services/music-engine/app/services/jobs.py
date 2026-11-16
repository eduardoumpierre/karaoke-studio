"""Fila de jobs: um único worker (a GPU faz um trabalho por vez), persistida no SQLite."""

from __future__ import annotations

import logging
import shutil
import sqlite3
import threading
from pathlib import Path

from ..config import Settings
from ..db import connect
from ..models.job import Job
from .analysis import AnalysisService
from .analyzer import Analyzer
from .audio_tools import AudioToolError, Cancelled, encode_flac, normalize_to_wav
from .library import LibraryError
from .separator import Separator

log = logging.getLogger("karaoke.jobs")

_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
_JOB_COLUMNS = (
    "id, song_id, kind, status, stage, progress, error, created_at, updated_at, "
    "(SELECT title FROM songs WHERE songs.id = jobs.song_id) AS song_title"
)
# Cada pista tem o seu worker: a separação usa a GPU e a análise só a CPU; uma não espera a outra.
LANES: tuple[tuple[str, ...], ...] = (("separate",), ("analyze",))
ACTIVE = ("queued", "running")


class JobError(LibraryError):
    code = "job_error"


class JobNotFound(JobError):
    status_code = 404
    code = "job_not_found"


class JobFinished(JobError):
    status_code = 409
    code = "job_finished"


class AlreadySeparated(JobError):
    status_code = 409
    code = "already_separated"


class SeparatorUnavailable(JobError):
    status_code = 503
    code = "separator_unavailable"


class JobStore:
    """Acesso ao banco para jobs. Cada instância usa a própria conexão."""

    def __init__(self, conn: sqlite3.Connection) -> None:
        self.conn = conn

    def get(self, job_id: int) -> Job:
        row = self.conn.execute(
            f"SELECT {_JOB_COLUMNS} FROM jobs WHERE id = ?", (job_id,)
        ).fetchone()
        if row is None:
            raise JobNotFound("Job não encontrado.")
        return Job(**dict(row))

    def list(self, active_only: bool = False, limit: int = 100) -> list[Job]:
        where = "WHERE status IN ('queued', 'running')" if active_only else ""
        rows = self.conn.execute(
            f"SELECT {_JOB_COLUMNS} FROM jobs {where} ORDER BY id DESC LIMIT ?", (limit,)
        ).fetchall()
        return [Job(**dict(r)) for r in rows]

    def active_for_song(self, song_id: int, kind: str | None = None) -> Job | None:
        sql = (
            f"SELECT {_JOB_COLUMNS} FROM jobs WHERE song_id = ? AND status IN ('queued', 'running')"
        )
        args: list[object] = [song_id]
        if kind:
            sql += " AND kind = ?"
            args.append(kind)
        row = self.conn.execute(sql + " ORDER BY id LIMIT 1", args).fetchone()
        return Job(**dict(row)) if row else None

    def create(self, song_id: int, kind: str, force: bool = False) -> Job:
        cur = self.conn.execute(
            "INSERT INTO jobs (song_id, kind, stage, force) VALUES (?, ?, 'na fila', ?)",
            (song_id, kind, int(force)),
        )
        self.conn.commit()
        return self.get(cur.lastrowid)  # type: ignore[arg-type]

    def claim_next(self, kinds: tuple[str, ...] | None = None) -> Job | None:
        where = "status = 'queued'"
        args: tuple[str, ...] = ()
        if kinds:
            where += f" AND kind IN ({', '.join('?' for _ in kinds)})"
            args = kinds
        row = self.conn.execute(
            f"SELECT id FROM jobs WHERE {where} ORDER BY id LIMIT 1", args
        ).fetchone()
        if row is None:
            return None
        self.conn.execute(
            f"UPDATE jobs SET status='running', stage='iniciando', started_at={_NOW}, "
            f"updated_at={_NOW} WHERE id = ?",
            (row["id"],),
        )
        self.conn.commit()
        return self.get(row["id"])

    def update_progress(self, job_id: int, stage: str, progress: float) -> None:
        self.conn.execute(
            f"UPDATE jobs SET stage = ?, progress = ?, updated_at = {_NOW} "
            "WHERE id = ? AND status = 'running'",
            (stage, max(0.0, min(progress, 1.0)), job_id),
        )
        self.conn.commit()

    def finish(self, job_id: int, status: str, error: str | None = None) -> None:
        progress = ", progress = 1" if status == "done" else ""
        self.conn.execute(
            f"UPDATE jobs SET status = ?, error = ?, stage = NULL, finished_at = {_NOW}, "
            f"updated_at = {_NOW}{progress} WHERE id = ?",
            (status, error, job_id),
        )
        self.conn.commit()

    def cancel_queued(self, job_id: int) -> bool:
        cur = self.conn.execute(
            f"UPDATE jobs SET status='cancelled', stage=NULL, finished_at={_NOW}, "
            f"updated_at={_NOW} WHERE id = ? AND status = 'queued'",
            (job_id,),
        )
        self.conn.commit()
        return cur.rowcount > 0

    def recover_interrupted(self) -> int:
        """Jobs que estavam rodando quando o app fechou não terminaram: marca como falhos."""
        cur = self.conn.execute(
            f"UPDATE jobs SET status='failed', stage=NULL, finished_at={_NOW}, updated_at={_NOW}, "
            "error='Interrompido: o aplicativo foi fechado durante o processamento.' "
            "WHERE status = 'running'"
        )
        self.conn.commit()
        return cur.rowcount


class JobRunner:
    """Thread única que executa os jobs na ordem em que foram criados."""

    def __init__(self, settings: Settings, separator: Separator, analyzer: Analyzer) -> None:
        self.settings = settings
        self.separator = separator
        self.analyzer = analyzer
        self._wakes = [threading.Event() for _ in LANES]
        self._stopping = threading.Event()
        self._lock = threading.Lock()
        self._cancel_events: dict[int, threading.Event] = {}
        self._threads: list[threading.Thread] = []

    # ------------------------------------------------------------ ciclo de vida
    def start(self) -> None:
        conn = connect(self.settings.db_path)
        try:
            n = JobStore(conn).recover_interrupted()
            if n:
                log.warning("%d job(s) interrompido(s) marcados como falhos", n)
        finally:
            conn.close()
        shutil.rmtree(self.settings.data_dir / "tmp", ignore_errors=True)
        for kinds, wake in zip(LANES, self._wakes, strict=True):
            thread = threading.Thread(
                target=self._loop, args=(kinds, wake), name=f"job-{kinds[0]}", daemon=True
            )
            thread.start()
            self._threads.append(thread)
            wake.set()  # pode haver jobs na fila de uma execução anterior

    def stop(self) -> None:
        self._stopping.set()
        with self._lock:
            for event in self._cancel_events.values():
                event.set()
        for wake in self._wakes:
            wake.set()
        for thread in self._threads:
            thread.join(timeout=15)

    def notify(self) -> None:
        for wake in self._wakes:
            wake.set()

    # ------------------------------------------------------------ cancelamento
    def cancel(self, job_id: int) -> Job:
        """Cancela um job: na fila, na hora; em execução, avisa o worker (que mata o processo)."""
        conn = connect(self.settings.db_path)
        try:
            store = JobStore(conn)
            with self._lock:
                job = store.get(job_id)
                if job.status not in ACTIVE:
                    raise JobFinished("Este job já terminou.")
                if not store.cancel_queued(job_id):
                    event = self._cancel_events.get(job_id)
                    if event:
                        event.set()
            return store.get(job_id)
        finally:
            conn.close()

    # ------------------------------------------------------------ worker
    def _loop(self, kinds: tuple[str, ...], wake: threading.Event) -> None:
        conn = connect(self.settings.db_path)
        store = JobStore(conn)
        try:
            while not self._stopping.is_set():
                with self._lock:
                    job = store.claim_next(kinds)
                    if job:
                        event = self._cancel_events[job.id] = threading.Event()
                if job is None:
                    wake.wait()
                    wake.clear()
                    continue
                try:
                    self._execute(store, job, event)
                finally:
                    with self._lock:
                        self._cancel_events.pop(job.id, None)
        finally:
            conn.close()

    def _execute(self, store: JobStore, job: Job, cancel: threading.Event) -> None:
        log.info("job %d (%s) iniciado, música %d", job.id, job.kind, job.song_id)
        try:
            if job.kind == "separate":
                self._separate(store, job, cancel)
            elif job.kind == "analyze":
                self._analyze(store, job, cancel)
            else:
                raise AudioToolError(f"Tipo de job desconhecido: {job.kind}")
        except Cancelled:
            log.info("job %d cancelado", job.id)
            store.finish(job.id, "cancelled")
        except (AudioToolError, LibraryError) as exc:
            log.error("job %d falhou: %s", job.id, exc)
            store.finish(job.id, "failed", str(exc))
        except Exception as exc:
            log.exception("job %d falhou de forma inesperada", job.id)
            store.finish(job.id, "failed", f"Erro inesperado: {exc}")
        else:
            log.info("job %d concluído", job.id)
            store.finish(job.id, "done")

    def _analyze(self, store: JobStore, job: Job, cancel: threading.Event) -> None:
        row = store.conn.execute("SELECT force FROM jobs WHERE id = ?", (job.id,)).fetchone()
        last = {"stage": "", "value": -1.0}

        def report(value: float, stage: str) -> None:
            if stage != last["stage"] or value - last["value"] >= 0.01:
                last.update(stage=stage, value=value)
                store.update_progress(job.id, stage, value)

        AnalysisService(store.conn, self.settings.data_dir, self.analyzer).run(
            job.song_id, bool(row and row["force"]), report, cancel
        )

    def _separate(self, store: JobStore, job: Job, cancel: threading.Event) -> None:
        row = store.conn.execute(
            "SELECT stored_path FROM songs WHERE id = ?", (job.song_id,)
        ).fetchone()
        if row is None:
            raise AudioToolError("A música foi removida antes do processamento.")
        original = self.settings.data_dir / row["stored_path"]
        song_dir = original.parent
        work = self.settings.data_dir / "tmp" / f"job-{job.id}"
        shutil.rmtree(work, ignore_errors=True)
        work.mkdir(parents=True)

        last = {"stage": "", "value": -1.0}

        def report(stage: str, value: float) -> None:
            # Só grava quando muda a etapa ou avança pelo menos 1%.
            if stage != last["stage"] or value - last["value"] >= 0.01:
                last.update(stage=stage, value=value)
                store.update_progress(job.id, stage, value)

        try:
            report("normalizando", 0.0)
            wav = work / "input.wav"
            normalize_to_wav(original, wav, cancel)
            # O Demucs só imprime progresso depois de carregar o modelo (e, na 1ª vez, baixá-lo).
            report("carregando modelo", 0.05)
            stems = self.separator.separate(
                wav, work / "out", lambda f: report("separando", 0.05 + 0.85 * f), cancel
            )
            report("convertendo", 0.9)
            staged = work / "stems"
            staged.mkdir()
            encode_flac(stems.vocals, staged / "vocals.flac", cancel)
            report("convertendo", 0.95)
            encode_flac(stems.instrumental, staged / "instrumental.flac", cancel)
            if cancel.is_set():
                raise Cancelled
            if not song_dir.is_dir():
                raise AudioToolError("A música foi removida durante o processamento.")
            final = song_dir / "stems"
            shutil.rmtree(final, ignore_errors=True)
            shutil.move(str(staged), str(final))
            store.conn.execute("UPDATE songs SET status = 'separated' WHERE id = ?", (job.song_id,))
            store.conn.commit()
        finally:
            shutil.rmtree(work, ignore_errors=True)


def stems_dir(settings: Settings, stored_path: str) -> Path:
    return (settings.data_dir / stored_path).parent / "stems"
