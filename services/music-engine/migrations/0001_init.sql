-- Esquema inicial: músicas, jobs (fila de processamento) e configurações.

CREATE TABLE songs (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    title            TEXT    NOT NULL,
    artist           TEXT    NOT NULL DEFAULT '',
    album            TEXT    NOT NULL DEFAULT '',
    duration_seconds REAL    NOT NULL,
    format           TEXT    NOT NULL,          -- mp3, wav, flac
    codec            TEXT    NOT NULL,
    sample_rate      INTEGER NOT NULL,
    channels         INTEGER NOT NULL,
    bit_rate         INTEGER,
    size_bytes       INTEGER NOT NULL,
    file_hash        TEXT    NOT NULL UNIQUE,   -- sha256 do arquivo original
    original_name    TEXT    NOT NULL,
    stored_path      TEXT    NOT NULL,          -- relativo à pasta de dados
    status           TEXT    NOT NULL DEFAULT 'imported',
    created_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE jobs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id    INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,               -- separate, analyze, lyrics...
    status     TEXT    NOT NULL DEFAULT 'queued',
    progress   REAL    NOT NULL DEFAULT 0,
    error      TEXT,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_jobs_song ON jobs(song_id);
CREATE INDEX idx_jobs_status ON jobs(status);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
