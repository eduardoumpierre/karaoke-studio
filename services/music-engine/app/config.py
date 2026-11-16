"""Configuração do serviço, lida das variáveis de ambiente que o Tauri define ao iniciá-lo."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

# Origens permitidas: a janela do Tauri e o servidor de desenvolvimento do Vite.
ALLOWED_ORIGINS = [
    "http://tauri.localhost",
    "https://tauri.localhost",
    "tauri://localhost",
    "http://localhost:1420",
    "http://127.0.0.1:1420",
]

TOKEN_HEADER = "X-Karaoke-Token"


def default_data_dir() -> Path:
    base = (
        os.environ.get("LOCALAPPDATA")
        or os.environ.get("XDG_DATA_HOME")
        or str(Path.home() / ".local" / "share")
    )
    return Path(base) / "KaraokeStudio"


@dataclass(frozen=True)
class Settings:
    token: str
    data_dir: Path
    warm_up: bool = False  # compila o numba em segundo plano ao iniciar (só no app real)

    @property
    def numba_cache_dir(self) -> Path:
        return self.data_dir / "numba-cache"

    @property
    def log_dir(self) -> Path:
        return self.data_dir / "logs"

    @property
    def db_path(self) -> Path:
        return self.data_dir / "library.db"

    @classmethod
    def from_env(cls) -> Settings:
        token = os.environ.get("KARAOKE_TOKEN", "")
        if len(token) < 16:
            raise RuntimeError("KARAOKE_TOKEN ausente ou curto demais (mínimo 16 caracteres).")
        data_dir = Path(os.environ.get("KARAOKE_DATA_DIR") or default_data_dir())
        return cls(token=token, data_dir=data_dir, warm_up=True)
