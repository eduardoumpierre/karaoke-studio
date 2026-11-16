# Karaoke Studio

Aplicacao desktop de karaoke local, com separacao de vocal e instrumental por IA,
letras sincronizadas, analise musical e processamento de voz em tempo real.

**Status:** Fase 0 (spikes e decisoes)

## Estrutura

- `apps/desktop`: interface (React + TypeScript + Vite) e nucleo Tauri/Rust (motor de audio)
- `services/music-engine`: backend Python (FastAPI, Demucs, FFmpeg, Librosa, SQLite)
- `packages/api-client`: tipos TypeScript gerados a partir do OpenAPI do backend
- `spikes`: prototipos descartaveis da Fase 0
- `docs`: arquitetura, desenvolvimento e decisoes (ADRs)

Veja `docs/PLANO-DE-DESENVOLVIMENTO.md` para o plano completo.
