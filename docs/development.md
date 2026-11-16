# Desenvolvimento

## Pré-requisitos (Windows 11)

Git, Node LTS, pnpm, Rust (rustup, com as Build Tools do Visual Studio C++), uv, FFmpeg. WebView2 já vem no Windows 11.

## Primeira vez

```powershell
cd C:\Users\umpie\projects\karaoke-studio
pnpm install
cd services\music-engine
uv sync
cd ..\..
```

## Rodar o app completo (Tauri + backend)

```powershell
cd apps\desktop
pnpm tauri dev
```

O Tauri sobe o Vite, abre a janela e inicia o `music-engine` com o Python de `services\music-engine\.venv`
(a porta é escolhida na hora e o token é gerado a cada execução). A janela mostra "Conectado" quando o `/health` responde.
Se o backend não subir, veja `%LOCALAPPDATA%\dev.karaokestudio.app\logs\`
(`karaoke-studio.log` do Tauri e `music-engine.log` do backend).

## Rodar só a UI no navegador (sem Tauri)

```powershell
# terminal 1: backend em porta fixa
cd services\music-engine
$env:KARAOKE_TOKEN = "token-de-desenvolvimento-1234"
$env:KARAOKE_PORT = "8765"
uv run python -m app

# terminal 2: UI
cd apps\desktop
$env:VITE_BACKEND_URL = "http://127.0.0.1:8765"
$env:VITE_BACKEND_TOKEN = "token-de-desenvolvimento-1234"
pnpm dev     # abra http://127.0.0.1:1420
```

## Verificações

```powershell
pnpm typecheck; pnpm test; pnpm lint; pnpm format:check      # UI
cd services\music-engine; uv run pytest; uv run ruff check .; uv run ruff format --check .
cd ..\..\apps\desktop\src-tauri; cargo test; cargo fmt --check
```

## Tipos da API

Depois de mudar rotas ou modelos do backend, veja `packages/api-client/README.md`.
