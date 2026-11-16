# Karaoke Studio

Aplicação desktop de karaokê local, com separação de vocal e instrumental por IA,
letras sincronizadas, análise musical e processamento de voz em tempo real.

**Status:** Fase 1 (fundação): a janela abre, o backend sobe junto e a UI mostra "Conectado".

## Estrutura

- `apps/desktop`: interface (React + TypeScript + Vite) e núcleo Tauri/Rust (motor de áudio)
- `services/music-engine`: backend Python (FastAPI, Demucs, FFmpeg, Librosa, SQLite)
- `packages/api-client`: tipos TypeScript gerados a partir do OpenAPI do backend
- `spikes`: protótipos descartáveis da Fase 0
- `docs`: arquitetura, desenvolvimento e decisões (ADRs)

Veja `docs/PLANO-DE-DESENVOLVIMENTO.md` para o plano completo e `docs/development.md` para rodar o projeto.
