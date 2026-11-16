import sqlite3

from app.db import migrate


def test_migrations_apply_once_and_create_tables(tmp_path):
    db = tmp_path / "x.db"
    first = migrate(db)
    assert first == [
        "0001_init.sql",
        "0002_jobs_stage.sql",
        "0003_lyrics.sql",
        "0004_analysis.sql",
        "0005_pitch.sql",
        "0006_job_force.sql",
    ]
    assert migrate(db) == []
    conn = sqlite3.connect(db)
    names = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    assert {"songs", "jobs", "settings", "schema_migrations"} <= names


def test_new_migration_files_are_picked_up(tmp_path):
    mig = tmp_path / "mig"
    mig.mkdir()
    (mig / "0001_a.sql").write_text("CREATE TABLE a (id INTEGER);")
    db = tmp_path / "y.db"
    assert migrate(db, mig) == ["0001_a.sql"]
    (mig / "0002_b.sql").write_text("CREATE TABLE b (id INTEGER);")
    assert migrate(db, mig) == ["0002_b.sql"]


def test_failed_migration_is_not_recorded(tmp_path):
    mig = tmp_path / "mig"
    mig.mkdir()
    (mig / "0001_bad.sql").write_text("CREATE TABLE ok (id INTEGER); SELECT nao_existe FROM nada;")
    db = tmp_path / "z.db"
    try:
        migrate(db, mig)
    except sqlite3.Error:
        pass
    conn = sqlite3.connect(db)
    applied = conn.execute("SELECT count(*) FROM schema_migrations").fetchone()[0]
    assert applied == 0
