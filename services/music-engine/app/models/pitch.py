from __future__ import annotations

from pydantic import BaseModel, Field

from .song import AudioFiles

MAX_SHIFT = 6


class Pitch(BaseModel):
    semitones: int = Field(description="Transposição atual, em semitons (0 = original)")
    rendered: list[int] = Field(default=[], description="Tons já prontos no disco")
    cache_bytes: int = 0


class PitchRequest(BaseModel):
    semitones: int = Field(ge=-MAX_SHIFT, le=MAX_SHIFT)


class PitchResult(BaseModel):
    pitch: Pitch
    audio: AudioFiles
