"""Mede cada etapa da análise de tom e BPM em um arquivo de música de verdade.

Uso: uv run python scripts/bench_analysis.py "C:\\caminho\\musica.mp3"
Mostra o andamento ao vivo. Roda duas vezes: a segunda mostra o tempo já aquecido.
Se travar, o arquivo analysis-worker-trace.log (na pasta do cache) mostra onde.
"""

from __future__ import annotations

import os
import sys
import threading
import time
from pathlib import Path


def run_once(analyzer, path: Path, duration: float):
    started = time.monotonic()
    state = {"stage": "", "since": started}

    def say(text: str) -> None:
        print(f"  {time.monotonic() - started:6.1f} s  {text}", flush=True)

    def on_progress(_fraction: float, stage: str) -> None:
        if stage != state["stage"]:
            state.update(stage=stage, since=time.monotonic())
            say(f"entrou em: {stage}")

    done = threading.Event()

    def heartbeat() -> None:
        while not done.wait(5):
            say(f"... ainda em “{state['stage']}” há {time.monotonic() - state['since']:.0f} s")

    threading.Thread(target=heartbeat, daemon=True).start()
    try:
        result = analyzer.analyze(path, duration, on_progress)
    finally:
        done.set()
    return time.monotonic() - started, result


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit('Uso: uv run python scripts/bench_analysis.py "musica.mp3"')
    from app.config import numba_cache_dir
    from app.services.analyzer import LibrosaAnalyzer
    from app.services.probe import probe

    path = Path(sys.argv[1])
    cache = numba_cache_dir()
    trace = cache.parent / "analysis-worker-trace.log"
    cache.mkdir(parents=True, exist_ok=True)
    trace.unlink(missing_ok=True)
    os.environ["KARAOKE_WORKER_TRACE"] = str(trace)
    duration = probe(path).duration_seconds
    print(f"Arquivo: {path.name} ({duration:.0f} s)\nCache do numba: {cache}", flush=True)
    analyzer = LibrosaAnalyzer(cache)
    try:
        for attempt in (1, 2):
            print(f"\nExecução {attempt}:", flush=True)
            total, result = run_once(analyzer, path, duration)
            print(f"  total: {total:.1f} s | worker: {result.timings}")
            print(f"  resultado: BPM {result.bpm}, tom {result.tonic} {result.mode}", flush=True)
    finally:
        analyzer.close()


if __name__ == "__main__":
    main()
