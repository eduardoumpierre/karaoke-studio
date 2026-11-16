"""SQLite com migrações simples: arquivos NNNN_nome.sql aplicados em ordem, uma vez cada."""

from __future__ import annotations

import sqlite3
from pathlib import Path

MIGRATIONS_DIR = Path(__file__).resolve().parents[2] / "migrations"


def connect(db_path: Path) -> sqlite3.Connection:
    # Cada requisição abre a própria conexão, mas o FastAPI pode abrir e usar a conexão em
    # threads diferentes do pool; como ela nunca é usada em paralelo, isso é seguro.
    conn = sqlite3.connect(db_path, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def migrate(db_path: Path, migrations_dir: Path = MIGRATIONS_DIR) -> list[str]:
    """Aplica as migrações pendentes. Devolve os nomes aplicados agora."""
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = connect(db_path)
    try:
        conn.execute("PRAGMA journal_mode = WAL")
        conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations ("
            "name TEXT PRIMARY KEY, applied_at TEXT NOT NULL "
            "DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
        )
        done = {r["name"] for r in conn.execute("SELECT name FROM schema_migrations")}
        applied: list[str] = []
        for sql_file in sorted(migrations_dir.glob("*.sql")):
            if sql_file.name in done:
                continue
            # executescript faz commit implícito; cada migração roda inteira ou falha com erro.
            conn.executescript(
                "BEGIN;\n" + sql_file.read_text(encoding="utf-8") + "\n"
                f"INSERT INTO schema_migrations(name) VALUES ('{sql_file.name}');\nCOMMIT;"
            )
            applied.append(sql_file.name)
        return applied
    finally:
        conn.close()
