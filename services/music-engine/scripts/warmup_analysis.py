"""Compila o numba (usado pelo Librosa) uma vez, deixando a análise de tom e BPM rápida.

Uso: uv run python scripts/warmup_analysis.py [PASTA_DO_CACHE]
Sem argumento usa o cache fixo do app (o mesmo que o servidor usa).
"""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> None:
    from app.config import numba_cache_dir
    from app.services.analyzer import LibrosaAnalyzer

    cache = Path(sys.argv[1]) if len(sys.argv) > 1 else numba_cache_dir()
    print(f"Cache do numba: {cache}\nCompilando (na primeira vez pode levar alguns minutos)...")
    analyzer = LibrosaAnalyzer(cache)
    try:
        analyzer.warm_up()
    finally:
        analyzer.close()
    print("Pronto. As análises seguintes serão rápidas.")


if __name__ == "__main__":
    main()
