"""Análise de andamento (BPM) e tonalidade com Librosa.

O resultado é uma estimativa: andamento pode sair em metade/dobro e tom pode confundir
relativas (Lá menor × Dó maior). Por isso a interface trata tudo como sugestão editável.
"""

from __future__ import annotations

import logging
import math
import multiprocessing
import os
import tempfile
import threading
import time
from collections.abc import Callable
from concurrent.futures import ProcessPoolExecutor
from concurrent.futures import TimeoutError as FutureTimeout
from concurrent.futures.process import BrokenProcessPool
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import numpy as np

from .audio_tools import Cancelled, ffmpeg_command, run_cancellable

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


_STAGE = None  # Value compartilhado com o processo pai (definido em _init_worker)


def _init_worker(stage) -> None:
    global _STAGE
    _STAGE = stage


def _warm_up_worker() -> float:
    """Roda no worker: força a compilação do numba com um áudio sintético curto (sem etapas)."""
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


def _set_stage(n: int) -> None:
    if _STAGE is not None:
        _STAGE.value = n


def _analyze_wav(wav: str) -> AnalysisResult:
    """Roda em um processo filho: o Librosa/numba compila e calcula sem tocar no GIL do servidor."""
    _set_stage(1)
    import librosa  # import tardio: carregar o numba leva alguns segundos
    import soundfile as sf

    y, sr = sf.read(wav, dtype="float32")
    _set_stage(2)
    tempo, _ = librosa.beat.beat_track(y=y, sr=sr)
    bpm = fold_bpm(float(np.atleast_1d(tempo)[0]))
    _set_stage(3)
    chroma = librosa.feature.chroma_cqt(y=y, sr=sr, hop_length=2048).mean(axis=1)
    tonic, mode, confidence = estimate_key(chroma)
    return AnalysisResult(
        bpm=round(bpm, 1) if bpm > 0 else None,
        tonic=tonic,
        mode=mode,
        confidence=confidence,
    )


class LibrosaAnalyzer:
    """Decodifica um trecho (até 2 min, a partir dos 15 s para pular introduções) e analisa.

    A parte pesada roda em um processo separado (spawn, um único worker, reaproveitado para
    manter o cache do numba aquecido). Em thread, a compilação do numba segurava o GIL e
    travava todas as rotas do servidor durante a análise.
    """

    TIMEOUT_S = 600.0
    POLL_S = 0.25

    def __init__(self, cache_dir: Path | None = None) -> None:
        if cache_dir is not None:
            # Cache de compilação do numba numa pasta fixa e gravável; o worker herda a variável.
            cache_dir.mkdir(parents=True, exist_ok=True)
            os.environ["NUMBA_CACHE_DIR"] = str(cache_dir)
        self._pool: ProcessPoolExecutor | None = None
        self._lock = threading.Lock()
        self._ctx = multiprocessing.get_context("spawn")
        self._stage = self._ctx.Value("i", 0, lock=False)  # sem lock: o worker pode ser morto

    def _executor(self) -> ProcessPoolExecutor:
        with self._lock:
            if self._pool is None:
                self._pool = ProcessPoolExecutor(
                    max_workers=1,
                    mp_context=self._ctx,
                    initializer=_init_worker,
                    initargs=(self._stage,),
                )
            return self._pool

    def warm_up(self) -> None:
        """Compila o numba agora (bloqueante). A primeira vez é lenta; depois fica em cache."""
        try:
            took = self._executor().submit(_warm_up_worker).result(self.TIMEOUT_S)
            log.info("aquecimento da análise concluído em %.1f s", took)
        except Exception:  # noqa: BLE001 - aquecer é só otimização
            log.warning("aquecimento da análise falhou", exc_info=True)

    def close(self) -> None:
        """Encerra o worker (também o que estiver no meio de uma análise)."""
        with self._lock:
            pool, self._pool = self._pool, None
        if pool is None:
            return
        procs = list(getattr(pool, "_processes", {}).values())
        pool.shutdown(wait=False, cancel_futures=True)
        for proc in procs:
            proc.terminate()

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
            cmd = [ffmpeg_command(), "-y", "-loglevel", "error"]
            if duration > MAX_SECONDS + 30:
                cmd += ["-ss", "15"]
            cmd += ["-i", str(path), "-t", str(MAX_SECONDS), "-vn", "-ac", "1"]
            cmd += ["-ar", str(SAMPLE_RATE), "-c:a", "pcm_f32le", str(wav)]
            run_cancellable(cmd, cancel, "Decodificação para análise")
            try:
                self._stage.value = 0
                future = self._executor().submit(_analyze_wav, str(wav))
                stage, stage_since = 0, time.monotonic()
                while True:
                    try:
                        result = future.result(self.POLL_S)
                        log.info("análise concluída em %.1f s", time.monotonic() - started)
                        return result
                    except FutureTimeout:
                        pass
                    now = time.monotonic()
                    if cancel.is_set():
                        self.close()  # mata o worker; o próximo pedido cria outro
                        raise Cancelled
                    if now - started > self.TIMEOUT_S:
                        self.close()  # descarta o worker preso; o próximo pedido cria outro
                        raise TimeoutError("A análise demorou demais e foi cancelada.")
                    if self._stage.value != stage:
                        stage, stage_since = self._stage.value, now
                        log.info(
                            "etapa %d (%s) após %.1f s", stage, STAGES[stage][0], now - started
                        )
                    report(*stage_progress(stage, now - stage_since))
            except BrokenProcessPool:
                self.close()  # o processo morreu: o próximo pedido cria outro
                raise
