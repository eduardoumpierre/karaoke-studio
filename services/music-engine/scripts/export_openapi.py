"""Gera o openapi.json que alimenta os tipos TypeScript (packages/api-client).

Uso (em services/music-engine):
    uv run python scripts/export_openapi.py ../../packages/api-client/openapi.json
"""

import json
import sys
import tempfile
from pathlib import Path

from app.config import Settings
from app.main import create_app

out = Path(sys.argv[1] if len(sys.argv) > 1 else "openapi.json")
app = create_app(Settings(token="x" * 16, data_dir=Path(tempfile.gettempdir()) / "ks-openapi"))
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(app.openapi(), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
print(f"OpenAPI gravado em {out}")
