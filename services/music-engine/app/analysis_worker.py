"""Processo de análise de tom e BPM: `python -m app.analysis_worker ARQUIVO.wav` (ou `--warmup`).

Protocolo na saída padrão, uma linha por mensagem:
  STAGE n          a etapa mudou (1 = carregando, 2 = andamento, 3 = tom)
  RESULT {json}    o resultado final
Erros vão para a saída de erros e o código de saída é diferente de zero.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import asdict


def _say(text: str) -> None:
    sys.stdout.write(text + "\n")
    sys.stdout.flush()


def main(argv: list[str]) -> int:
    trace = os.environ.get("KARAOKE_WORKER_TRACE")
    if trace:  # diagnóstico: a cada 15 s grava onde o processo está parado
        import faulthandler

        faulthandler.dump_traceback_later(15, repeat=True, file=open(trace, "a"))  # noqa: SIM115
    from .services.analyzer import analyze_wav, warm_up_clip

    if argv[:1] == ["--warmup"]:
        took = warm_up_clip()
        _say(
            "RESULT "
            + json.dumps(
                {
                    "bpm": 0.0,
                    "tonic": 0,
                    "mode": "major",
                    "confidence": 0.0,
                    "timings": {"aquecimento": took},
                }
            )
        )
        return 0
    if len(argv) != 1:
        sys.stderr.write("uso: python -m app.analysis_worker ARQUIVO.wav | --warmup\n")
        return 2
    result = analyze_wav(argv[0], lambda n: _say(f"STAGE {n}"))
    _say("RESULT " + json.dumps(asdict(result)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
