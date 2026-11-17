"""Separação voz/instrumental. A interface permite trocar o Demucs por um falso nos testes."""

from __future__ import annotations

import importlib.util
import logging
import os
import re
import subprocess
import sys
import threading
import time
from collections import deque
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from .audio_tools import NO_WINDOW, AudioToolError, Cancelled, stop_process


def default_device() -> str | None:
    """Dispositivo do Demucs quando KARAOKE_DEVICE não está definido.

    No macOS o Demucs escolhe a GPU (MPS) sozinho, mas o htdemucs falha nela com o
    torch 2.5.1 ("Output channels > 65536 not supported at the MPS device"), e
    PYTORCH_ENABLE_MPS_FALLBACK não resolve. Então usamos a CPU. Nos outros sistemas
    deixamos o Demucs decidir (CUDA quando houver).
    """
    return "cpu" if sys.platform == "darwin" else None


log = logging.getLogger("karaoke.demucs")

ProgressFn = Callable[[float], None]


@dataclass(frozen=True)
class Stems:
    vocals: Path
    instrumental: Path


class Separator(Protocol):
    def unavailable_reason(self) -> str | None:
        """None se pronto para uso; senão, o motivo (mostrado ao usuário)."""

    def separate(
        self, wav: Path, out_dir: Path, on_progress: ProgressFn, cancel: threading.Event
    ) -> Stems: ...


_PERCENT = re.compile(r"(\d{1,3})%\|")


def parse_progress(line: str) -> float | None:
    """Extrai a fração (0 a 1) de uma linha de barra de progresso do tqdm; None se não houver."""
    matches = _PERCENT.findall(line)
    if not matches:
        return None
    return min(int(matches[-1]), 100) / 100


class DemucsSeparator:
    """Roda `python -m demucs` num processo à parte: dá para cancelar de verdade (matando o
    processo) e a memória da GPU é devolvida ao terminar."""

    model = "htdemucs"

    def __init__(self, data_dir: Path, device: str | None = None) -> None:
        self.data_dir = data_dir
        self.device = device or os.environ.get("KARAOKE_DEVICE") or default_device()

    def unavailable_reason(self) -> str | None:
        if importlib.util.find_spec("demucs") is None:
            return "O Demucs não está instalado. Em services/music-engine, rode: uv sync --group ml"
        return None

    def command(self, wav: Path, out_dir: Path) -> list[str]:
        cmd = [sys.executable, "-m", "demucs", "-n", self.model, "--two-stems=vocals", "--float32"]
        if self.device:
            cmd += ["-d", self.device]
        return [*cmd, "-o", str(out_dir), str(wav)]

    def separate(
        self, wav: Path, out_dir: Path, on_progress: ProgressFn, cancel: threading.Event
    ) -> Stems:
        env = dict(os.environ)
        # Os pesos do modelo (~80 MB) ficam na pasta de dados do app, não no perfil do usuário.
        env["TORCH_HOME"] = str(self.data_dir / "models")
        env["PYTHONUNBUFFERED"] = "1"
        cmd = self.command(wav, out_dir)
        log.info("iniciando Demucs: %s", " ".join(cmd))
        proc = subprocess.Popen(
            cmd,
            stdin=subprocess.DEVNULL,  # não herda o stdin do sidecar (o pipe do Tauri)
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=env,
            creationflags=NO_WINDOW,
        )
        tail: deque[str] = deque(maxlen=6)
        started = time.monotonic()
        last_output = [started]
        last_logged = [-1]

        def handle(raw: bytes) -> None:
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                return
            last_output[0] = time.monotonic()
            frac = parse_progress(line)
            if frac is not None:
                if int(frac * 100) // 10 != last_logged[0]:  # um log a cada 10%
                    last_logged[0] = int(frac * 100) // 10
                    log.info("Demucs: %d%% (%.0f s)", frac * 100, time.monotonic() - started)
                on_progress(frac)
            else:
                log.info("Demucs: %s", line)
                tail.append(line)

        def read_stderr() -> None:
            # O tqdm separa as atualizações com \r, então lemos bytes e cortamos em \r e \n.
            assert proc.stderr is not None
            buf = b""
            while chunk := proc.stderr.read1(4096):  # type: ignore[attr-defined]
                *lines, buf = re.split(rb"[\r\n]", buf + chunk)
                for raw in lines:
                    handle(raw)
            handle(buf)  # a última atualização pode vir sem quebra de linha

        reader = threading.Thread(target=read_stderr, daemon=True)
        reader.start()
        while proc.poll() is None:
            if cancel.is_set():
                stop_process(proc)
                raise Cancelled
            if time.monotonic() - last_output[0] > 20:
                log.info(
                    "Demucs sem saída há %.0f s (processo %d vivo, %.0f s no total)",
                    time.monotonic() - last_output[0],
                    proc.pid,
                    time.monotonic() - started,
                )
                last_output[0] = time.monotonic()
            time.sleep(0.1)
        reader.join(timeout=5)
        log.info(
            "Demucs terminou com código %s em %.0f s", proc.returncode, time.monotonic() - started
        )
        if proc.returncode != 0:
            detail = " | ".join(tail) or f"código {proc.returncode}"
            raise AudioToolError(f"O Demucs falhou: {detail}")
        vocals = next(out_dir.rglob("vocals.wav"), None)
        instrumental = next(out_dir.rglob("no_vocals.wav"), None)
        if vocals is None or instrumental is None:
            raise AudioToolError("O Demucs terminou, mas não gerou os arquivos esperados.")
        return Stems(vocals, instrumental)
