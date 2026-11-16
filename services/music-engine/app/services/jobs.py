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
from .audio_tools import AudioToolError, Cancelled, encode_flac, normalize_to_wav
from .library import LibraryError
from .separator import Separator

log = logging.getLogger("karaoke.jobs")

_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
_JOB_COLUMNS = "id, song_id, kind, status, stage, progress, error, created_at, updated_at"
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

    def create(self, song_id: int, kind: str) -> Job:
        cur = self.conn.execute(
            "INSERT INTO jobs (song_id, kind, stage) VALUES (?, ?, 'na fila')", (song_id, kind)
        )
        self.conn.commit()
        return self.get(cur.lastrowid)  # type: ignore[arg-type]

    def claim_next(self) -> Job | None:
        row = self.conn.execute(
            "SELECT id FROM jobs WHERE status = 'queued' ORDER BY id LIMIT 1"
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

    def __init__(self, settings: Settings, separator: Separator) -> None:
        self.settings = settings
        self.separator = separator
        self._wake = threading.Event()
        self._stopping = threading.Event()
        self._lock = threading.Lock()
        self._cancel_events: dict[int, threading.Event] = {}
        self._thread: threading.Thread | None = None

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
        self._thread = threading.Thread(target=self._loop, name="job-runner", daemon=True)
        self._thread.start()
        self._wake.set()  # pode haver jobs na fila de uma execução anterior

    def stop(self) -> None:
        self._stopping.set()
        with self._lock:
            for event in self._cancel_events.values():
                event.set()
        self._wake.set()
        if self._thread:
            self._thread.join(timeout=15)

    def notify(self) -> None:
        self._wake.set()

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
    def _loop(self) -> None:
        conn = connect(self.settings.db_path)
        store = JobStore(conn)
        try:
            while not self._stopping.is_set():
                with self._lock:
                    job = store.claim_next()
                    if job:
                        event = self._cancel_events[job.id] = threading.Event()
                if job is None:
                    self._wake.wait()
                    self._wake.clear()
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
            if job.kind != "separate":
                raise AudioToolError(f"Tipo de job desconhecido: {job.kind}")
            self._separate(store, job, cancel)
        except Cancelled:
            log.info("job %d cancelado", job.id)
            store.finish(job.id, "cancelled")
        except AudioToolError as exc:
            log.error("job %d falhou: %s", job.id, exc)
            store.finish(job.id, "failed", str(exc))
        except Exception as exc:
            log.exception("job %d falhou de forma inesperada", job.id)
            store.finish(job.id, "failed", f"Erro inesperado: {exc}")
        else:
            log.info("job %d concluído", job.id)
            store.finish(job.id, "done")

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
