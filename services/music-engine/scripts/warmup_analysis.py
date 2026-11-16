"""Compila o numba (usado pelo Librosa) uma vez, deixando a análise de tom e BPM rápida.

Uso: uv run python scripts/warmup_analysis.py [PASTA_DE_DADOS]
Sem argumento usa a pasta de dados padrão do app (a mesma do servidor).
"""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> None:
    from app.config import default_data_dir
    from app.services.analyzer import LibrosaAnalyzer

    data_dir = (
        Path(sys.argv[1])
        if len(sys.argv) > 1
        else Path(os.environ.get("KARAOKE_DATA_DIR") or default_data_dir())
    )
    cache = data_dir / "numba-cache"
    print(f"Cache do numba: {cache}\nCompilando (na primeira vez pode levar alguns minutos)...")
    analyzer = LibrosaAnalyzer(cache)
    try:
        analyzer.warm_up()
    finally:
        analyzer.close()
    print("Pronto. As análises seguintes serão rápidas.")


if __name__ == "__main__":
    main()
