-- Etapa atual e horários dos jobs (para mostrar "normalizando", "separando"... na tela).

ALTER TABLE jobs ADD COLUMN stage       TEXT;
ALTER TABLE jobs ADD COLUMN started_at  TEXT;
ALTER TABLE jobs ADD COLUMN finished_at TEXT;
