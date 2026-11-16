"""Análise de andamento (BPM) e tonalidade com Librosa.

O resultado é uma estimativa: andamento pode sair em metade/dobro e tom pode confundir
relativas (Lá menor × Dó maior). Por isso a interface trata tudo como sugestão editável.
"""

from __future__ import annotations

import json
import logging
import math
import os
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import numpy as np

from .audio_tools import NO_WINDOW, Cancelled, ffmpeg_command, kill_tree, run_cancellable

# Perfis de Krumhansl-Schmuckler (tonalidade de referência = Dó / Lá menor).
MAJOR_PROFILE = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
MINOR_PROFILE = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])

SAMPLE_RATE = 22050
MAX_SECONDS = 120.0
BPM_MIN, BPM_MAX = 70.0, 180.0


@dataclass(frozen=True)
class AnalysisResult:
    bpm: float | None  # None = não deu para detectar o andamento
    tonic: int
    mode: str  # major | minor
    confidence: float
    # Segundos gastos em cada etapa do worker (diagnóstico de desempenho).
    timings: dict[str, float] | None = None


log = logging.getLogger("karaoke.analysis")

# Etapas do worker: (rótulo, início, fim, constante de tempo em s). A etapa só muda quando o worker
# avisa; dentro dela a barra se aproxima do fim aos poucos, sem nunca chegar antes do aviso.
STAGES: tuple[tuple[str, float, float, float], ...] = (
    ("Preparando o áudio", 0.00, 0.10, 3.0),
    ("Carregando o analisador", 0.10, 0.30, 6.0),
    ("Detectando o andamento", 0.30, 0.70, 8.0),
    ("Detectando o tom", 0.70, 0.97, 6.0),
)

# progress(fração 0–1, rótulo da etapa)
ProgressCallback = Callable[[float, str], None]


def stage_progress(stage: int, elapsed: float) -> tuple[float, str]:
    """Fração geral e rótulo para a etapa `stage` (0…) depois de `elapsed` segundos nela."""
    label, start, end, tau = STAGES[max(0, min(stage, len(STAGES) - 1))]
    return start + (end - start) * 0.95 * (1 - math.exp(-max(0.0, elapsed) / tau)), label


class Analyzer(Protocol):
    def analyze(
        self,
        path: Path,
        duration: float,
        progress: ProgressCallback | None = None,
        cancel: threading.Event | None = None,
    ) -> AnalysisResult: ...


def fold_bpm(bpm: float) -> float:
    """Leva o andamento para a faixa 70–180 (evita metade/dobro absurdos)."""
    if bpm <= 0:
        return bpm
    while bpm < BPM_MIN:
        bpm *= 2
    while bpm > BPM_MAX:
        bpm /= 2
    return bpm


def estimate_key(chroma: np.ndarray) -> tuple[int, str, float]:
    """Tom de um vetor de 12 energias (Dó…Si): maior correlação com os perfis de Krumhansl."""
    chroma = np.asarray(chroma, dtype=float)
    if chroma.shape != (12,) or np.std(chroma) < 1e-9:
        return 0, "major", 0.0
    best = (0, "major", -2.0)
    for mode, profile in (("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)):
        for tonic in range(12):
            r = float(np.corrcoef(chroma, np.roll(profile, tonic))[0, 1])
            if r > best[2]:
                best = (tonic, mode, r)
    tonic, mode, r = best
    return tonic, mode, max(0.0, min(1.0, r))


def analyze_wav(wav: str, on_stage: Callable[[int], None] | None = None) -> AnalysisResult:
    """O cálculo em si (roda no processo de análise, nunca no servidor)."""
    stage = on_stage or (lambda _n: None)
    t0 = time.monotonic()
    stage(1)
    import librosa  # import tardio: carregar o numba leva alguns segundos
    import soundfile as sf

    y, sr = sf.read(wav, dtype="float32")
    t1 = time.monotonic()
    stage(2)
    tempo, _ = librosa.beat.beat_track(y=y, sr=sr)
    bpm = fold_bpm(float(np.atleast_1d(tempo)[0]))
    t2 = time.monotonic()
    stage(3)
    chroma = librosa.feature.chroma_cqt(y=y, sr=sr, hop_length=2048).mean(axis=1)
    tonic, mode, confidence = estimate_key(chroma)
    t3 = time.monotonic()
    return AnalysisResult(
        timings={"carregar": t1 - t0, "andamento": t2 - t1, "tom": t3 - t2},
        bpm=round(bpm, 1) if bpm > 0 else None,
        tonic=tonic,
        mode=mode,
        confidence=confidence,
    )


def warm_up_clip() -> float:
    """Força a compilação do numba com um áudio sintético curto; devolve os segundos gastos."""
    started = time.monotonic()
    import librosa

    sr = SAMPLE_RATE
    t = np.arange(sr * 6) / sr
    y = (
        0.3 * np.sin(2 * np.pi * 220 * t)
        + librosa.clicks(times=np.arange(0, 6, 0.5), sr=sr, length=len(t))
    ).astype("float32")
    librosa.beat.beat_track(y=y, sr=sr)
    librosa.feature.chroma_cqt(y=y, sr=sr, hop_length=2048)
    return time.monotonic() - started


ENGINE_DIR = Path(__file__).resolve().parents[2]  # onde fica o pacote `app`


class WorkerFailed(RuntimeError):
    """O processo de análise terminou sem entregar um resultado."""


class LibrosaAnalyzer:
    """Decodifica um trecho (até 2 min, a partir dos 15 s para pular introduções) e analisa.

    O cálculo roda em um processo próprio (`python -m app.analysis_worker`), iniciado com
    `subprocess`: sem entrada, saída lida linha a linha e erros capturados. Em thread, a
    compilação do numba segurava o GIL e travava o servidor; com `multiprocessing` (spawn), o
    processo nunca chegou a rodar dentro do app no Windows. O cache do numba em disco faz o
    custo de compilação acontecer só uma vez.
    """

    TIMEOUT_S = 600.0
    POLL_S = 0.25

    def __init__(self, cache_dir: Path | None = None) -> None:
        self._env = dict(os.environ, PYTHONUNBUFFERED="1", PYTHONIOENCODING="utf-8")
        if cache_dir is not None:
            # Cache de compilação do numba numa pasta fixa e gravável.
            cache_dir.mkdir(parents=True, exist_ok=True)
            self._env["NUMBA_CACHE_DIR"] = str(cache_dir)
        self._lock = threading.Lock()
        self._procs: set[subprocess.Popen] = set()
        self._warming = threading.Event()  # aquecimento em andamento: a análise espera por ele

    # ------------------------------------------------------------------ processo
    def _run(
        self,
        args: list[str],
        cancel: threading.Event,
        on_stage: Callable[[int], None] | None = None,
    ) -> dict:
        """Roda o worker até o fim e devolve o JSON do RESULT."""
        out: dict = {}

        def read(stream) -> None:
            for line in stream:
                key, _, value = line.strip().partition(" ")
                if key == "STAGE" and on_stage:
                    on_stage(int(value))
                elif key == "RESULT":
                    out.update(json.loads(value))

        with tempfile.TemporaryFile() as err:
            cmd = [sys.executable, "-m", "app.analysis_worker", *args]
            proc = subprocess.Popen(  # noqa: S603
                cmd,
                cwd=ENGINE_DIR,
                env=self._env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=err,
                text=True,
                encoding="utf-8",
                creationflags=NO_WINDOW,
            )
            with self._lock:
                self._procs.add(proc)
            reader = threading.Thread(target=read, args=(proc.stdout,), daemon=True)
            reader.start()
            started = time.monotonic()
            try:
                while proc.poll() is None:
                    if cancel.is_set():
                        kill_tree(proc)
                        raise Cancelled
                    if time.monotonic() - started > self.TIMEOUT_S:
                        kill_tree(proc)
                        raise TimeoutError("A análise demorou demais e foi cancelada.")
                    time.sleep(0.05)
                reader.join(2)
            finally:
                with self._lock:
                    self._procs.discard(proc)
                if proc.poll() is None:
                    kill_tree(proc)
            if proc.returncode != 0 or "bpm" not in out:
                err.seek(0)
                tail = err.read().decode("utf-8", "replace").strip().splitlines()[-4:]
                raise WorkerFailed(
                    f"A análise terminou sem resultado (código {proc.returncode}): "
                    f"{' | '.join(tail) or 'sem mensagem'}"
                )
        return out

    def close(self) -> None:
        """Encerra qualquer análise em andamento."""
        with self._lock:
            procs = list(self._procs)
        for proc in procs:
            kill_tree(proc)

    def warm_up(self) -> None:
        """Compila o numba agora (bloqueante). A primeira vez é lenta; depois fica em cache."""
        self._warming.set()
        try:
            started = time.monotonic()
            self._run(["--warmup"], threading.Event())
            log.info("aquecimento da análise concluído em %.1f s", time.monotonic() - started)
        except Exception:  # noqa: BLE001 - aquecer é só otimização
            log.warning("aquecimento da análise falhou", exc_info=True)
        finally:
            self._warming.clear()

    # ------------------------------------------------------------------ análise
    def analyze(
        self,
        path: Path,
        duration: float,
        progress: ProgressCallback | None = None,
        cancel: threading.Event | None = None,
    ) -> AnalysisResult:
        report = progress or (lambda _f, _s: None)
        cancel = cancel or threading.Event()
        report(*stage_progress(0, 0.0))
        started = time.monotonic()
        with tempfile.TemporaryDirectory() as tmp:
            wav = Path(tmp) / "mono.wav"
            cmd = [ffmpeg_command(), "-y", "-nostdin", "-loglevel", "error"]
            if duration > MAX_SECONDS + 30:
                cmd += ["-ss", "15"]
            cmd += ["-i", str(path), "-t", str(MAX_SECONDS), "-vn", "-ac", "1"]
            cmd += ["-ar", str(SAMPLE_RATE), "-c:a", "pcm_f32le", str(wav)]
            run_cancellable(cmd, cancel, "Decodificação para análise")
            decoded_at = time.monotonic()
            log.info("áudio decodificado em %.1f s", decoded_at - started)

            # Compilar a mesma coisa duas vezes ao mesmo tempo só gasta CPU: espera o aquecimento.
            while self._warming.is_set():
                if cancel.is_set():
                    raise Cancelled
                if time.monotonic() - started > self.TIMEOUT_S:
                    raise TimeoutError("A análise demorou demais e foi cancelada.")
                report(*stage_progress(1, time.monotonic() - started))
                time.sleep(self.POLL_S)

            current = {"stage": 0, "since": time.monotonic()}
            lock = threading.Lock()

            def on_stage(n: int) -> None:
                with lock:
                    current.update(stage=n, since=time.monotonic())
                log.info("etapa %d (%s) após %.1f s", n, STAGES[n][0], time.monotonic() - started)

            # A barra anda sozinha enquanto o worker trabalha (a etapa só muda quando ele avisa).
            done = threading.Event()

            def tick() -> None:
                while not done.wait(self.POLL_S):
                    with lock:
                        stage, since = current["stage"], current["since"]
                    report(*stage_progress(stage, time.monotonic() - since))

            ticker = threading.Thread(target=tick, daemon=True)
            ticker.start()
            try:
                data = self._run([str(wav)], cancel, on_stage)
            finally:
                done.set()
                ticker.join(1)
        result = AnalysisResult(**data)
        log.info(
            "análise concluída em %.1f s (worker: %s)",
            time.monotonic() - started,
            ", ".join(f"{k} {v:.1f} s" for k, v in (result.timings or {}).items()),
        )
        return result
