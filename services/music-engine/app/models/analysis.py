from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field


class Analysis(BaseModel):
    # none = ainda não analisada
    status: Literal["none", "done"]
    bpm: float | None = None
    bpm_manual: bool = False
    key_tonic: int | None = Field(default=None, description="0 = Dó … 11 = Si")
    key_mode: Literal["major", "minor"] | None = None
    key_confidence: float | None = None
    key_manual: bool = False
    analyzed_at: str | None = Field(
        default=None, description="Quando a última análise terminou (UTC)"
    )
    # Preenchidos enquanto uma análise está em andamento.
    running: bool = False
    progress: float | None = Field(default=None, ge=0, le=1)
    stage: str | None = None
    elapsed_seconds: float | None = None
    queued: bool = False
    error: str | None = Field(default=None, description="Motivo da última análise ter falhado")


class AnalysisEdit(BaseModel):
    """Só os campos enviados mudam; ao editar, ficam marcados como definidos pelo usuário."""

    bpm: float | None = Field(default=None, ge=30, le=300)
    key_tonic: int | None = Field(default=None, ge=0, le=11)
    key_mode: Literal["major", "minor"] | None = None
