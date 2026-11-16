-- Letras por música (cache do LRCLIB ou coladas pelo usuário).

CREATE TABLE lyrics (
    song_id        INTEGER PRIMARY KEY REFERENCES songs(id) ON DELETE CASCADE,
    status         TEXT    NOT NULL,           -- found | not_found
    synced_lrc     TEXT,                       -- texto LRC com tempos
    plain          TEXT,                       -- letra sem tempos
    source         TEXT,                       -- lrclib | manual
    matched_title  TEXT,
    matched_artist TEXT,
    offset_ms      INTEGER NOT NULL DEFAULT 0, -- ajuste manual: positivo atrasa a letra
    fetched_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
