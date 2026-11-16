-- Transposição escolhida por música (em semitons, 0 = tom original).
-- Os arquivos já renderizados ficam em songs/<hash>/pitch/<+N>/ e podem ser apagados à vontade.

CREATE TABLE song_pitch (
    song_id   INTEGER PRIMARY KEY REFERENCES songs(id) ON DELETE CASCADE,
    semitones INTEGER NOT NULL DEFAULT 0
);
