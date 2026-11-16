"""Análise por música: roda o analisador, guarda no SQLite e aceita correções manuais."""

from __future__ import annotations

import sqlite3
import threading

from ..models.analysis import Analysis, AnalysisEdit
from .analyzer import Analyzer, ProgressCallback
from .audio_tools import AudioToolError, Cancelled
from .library import FileMissing, LibraryError, SongNotFound


class AnalysisFailed(LibraryError):
    status_code = 500
    code = "analysis_failed"


class AnalysisService:
    def __init__(self, conn: sqlite3.Connection, data_dir, analyzer: Analyzer) -> None:
        self.conn = conn
        self.data_dir = data_dir
        self.analyzer = analyzer

    def _job_fields(self, song_id: int) -> dict:
        """Estado da análise na fila de jobs: na fila/rodando, ou o motivo da última falha."""
        active = self.conn.execute(
            "SELECT status, stage, progress, "
            "(julianday('now') - julianday(COALESCE(started_at, created_at))) * 86400 AS elapsed "
            "FROM jobs WHERE song_id = ? AND kind = 'analyze' AND status IN ('queued', 'running') "
            "ORDER BY id LIMIT 1",
            (song_id,),
        ).fetchone()
        if active is not None:
            queued = active["status"] == "queued"
            return {
                "running": True,
                "queued": queued,
                "progress": 0.0 if queued else active["progress"],
                "stage": "Na fila" if queued else (active["stage"] or "Iniciando"),
                "elapsed_seconds": round(max(0.0, active["elapsed"] or 0.0), 1),
            }
        last = self.conn.execute(
            "SELECT status, error FROM jobs WHERE song_id = ? AND kind = 'analyze' "
            "ORDER BY id DESC LIMIT 1",
            (song_id,),
        ).fetchone()
        if last is not None and last["status"] == "failed":
            return {"error": last["error"] or "A análise falhou."}
        return {}

    def _song(self, song_id: int) -> sqlite3.Row:
        row = self.conn.execute(
            "SELECT stored_path, duration_seconds FROM songs WHERE id = ?", (song_id,)
        ).fetchone()
        if row is None:
            raise SongNotFound("Música não encontrada.")
        return row

    def get(self, song_id: int) -> Analysis:
        self._song(song_id)
        row = self.conn.execute("SELECT * FROM analysis WHERE song_id = ?", (song_id,)).fetchone()
        job = self._job_fields(song_id)
        if row is None or row["bpm"] is None and row["key_tonic"] is None:
            return Analysis(status="none", **job)
        return Analysis(
            status="done",
            bpm=row["bpm"],
            bpm_manual=bool(row["bpm_manual"]),
            key_tonic=row["key_tonic"],
            key_mode=row["key_mode"],
            key_confidence=row["key_confidence"],
            key_manual=bool(row["key_manual"]),
            analyzed_at=row["analyzed_at"],
            **{k: v for k, v in job.items() if k != "error"},
        )

    def run(
        self,
        song_id: int,
        force: bool = False,
        progress: ProgressCallback | None = None,
        cancel: threading.Event | None = None,
    ) -> Analysis:
        """Analisa e grava (chamado pelo worker de jobs). Edições manuais valem, salvo `force`."""
        song = self._song(song_id)
        path = self.data_dir / song["stored_path"]
        if not path.is_file():
            raise FileMissing("O arquivo da música não está mais na pasta de dados.", song_id)
        try:
            result = self.analyzer.analyze(path, song["duration_seconds"], progress, cancel)
        except Cancelled:
            raise
        except AudioToolError as exc:
            raise AnalysisFailed(str(exc), song_id) from exc
        except Exception as exc:  # librosa/numba: qualquer falha vira erro amigável
            raise AnalysisFailed(f"Não foi possível analisar a música: {exc}", song_id) from exc

        current = self.conn.execute(
            "SELECT bpm_manual, key_manual FROM analysis WHERE song_id = ?", (song_id,)
        ).fetchone()
        keep_bpm = bool(current and current["bpm_manual"]) and not force
        keep_key = bool(current and current["key_manual"]) and not force
        if current is None:
            self.conn.execute(
                "INSERT INTO analysis (song_id, bpm, key_tonic, key_mode, key_confidence) "
                "VALUES (?, ?, ?, ?, ?)",
                (song_id, result.bpm, result.tonic, result.mode, result.confidence),
            )
        else:
            if not keep_bpm:
                self.conn.execute(
                    "UPDATE analysis SET bpm = ?, bpm_manual = 0 WHERE song_id = ?",
                    (result.bpm, song_id),
                )
            if not keep_key:
                self.conn.execute(
                    "UPDATE analysis SET key_tonic = ?, key_mode = ?, key_confidence = ?, "
                    "key_manual = 0 WHERE song_id = ?",
                    (result.tonic, result.mode, result.confidence, song_id),
                )
            self.conn.execute(
                "UPDATE analysis SET analyzed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') "
                "WHERE song_id = ?",
                (song_id,),
            )
        self.conn.commit()
        return self.get(song_id)

    def edit(self, song_id: int, edit: AnalysisEdit) -> Analysis:
        self._song(song_id)
        self.conn.execute("INSERT OR IGNORE INTO analysis (song_id) VALUES (?)", (song_id,))
        if edit.bpm is not None:
            self.conn.execute(
                "UPDATE analysis SET bpm = ?, bpm_manual = 1 WHERE song_id = ?",
                (round(edit.bpm, 1), song_id),
            )
        if edit.key_tonic is not None or edit.key_mode is not None:
            row = self.conn.execute(
                "SELECT key_tonic, key_mode FROM analysis WHERE song_id = ?", (song_id,)
            ).fetchone()
            tonic = edit.key_tonic if edit.key_tonic is not None else (row["key_tonic"] or 0)
            mode = edit.key_mode or row["key_mode"] or "major"
            self.conn.execute(
                "UPDATE analysis SET key_tonic = ?, key_mode = ?, key_manual = 1, "
                "key_confidence = NULL WHERE song_id = ?",
                (tonic, mode, song_id),
            )
        self.conn.commit()
        return self.get(song_id)
