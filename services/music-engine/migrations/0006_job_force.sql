-- Análise refeita "descartando minhas edições" (força a sobrescrever BPM/tom definidos à mão).
ALTER TABLE jobs ADD COLUMN force INTEGER NOT NULL DEFAULT 0;
