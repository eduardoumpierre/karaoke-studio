-- Andamento (BPM) e tonalidade por música. Valores automáticos são sugestões;
-- os marcados como manuais foram definidos pelo usuário e não são sobrescritos.

CREATE TABLE analysis (
    song_id        INTEGER PRIMARY KEY REFERENCES songs(id) ON DELETE CASCADE,
    bpm            REAL,
    bpm_manual     INTEGER NOT NULL DEFAULT 0,
    key_tonic      INTEGER,                    -- 0 = Dó ... 11 = Si
    key_mode       TEXT,                       -- major | minor
    key_confidence REAL,                       -- 0 a 1 (só para o automático)
    key_manual     INTEGER NOT NULL DEFAULT 0,
    analyzed_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
