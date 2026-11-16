# Karaoke Studio — Revisão do Projeto e Plano de Desenvolvimento

**Base:** `karaoke-studio.txt` (plano inicial)
**Local do projeto:** `C:\Users\umpie\projects\karaoke-studio`
**Sistema de desenvolvimento:** Windows 11 (macOS entra depois, na Fase 7)

---

## 1. Resumo da revisão

O plano original é sólido: o escopo é claro, a separação entre processamento pesado (Music Engine) e tempo real (Audio Engine) está correta, e a regra de começar com arquivos locais evita problemas de direitos autorais. A ordem das sprints também é boa: primeiro o fluxo importar → separar → tocar, depois letras e microfone.

Pontos que eu mudaria antes de começar:

| # | Ponto | Risco | Recomendação |
|---|---|---|---|
| 1 | **JUCE/C++ como motor de áudio** | Licença (GPL ou comercial paga se o app for distribuído fechado), build C++ nos dois SOs, integração extra com o Tauri | Usar **Rust dentro do próprio Tauri** (`cpal` + `symphonia`) como motor de áudio. Manter JUCE só se um spike mostrar que o DSP vocal precisa dele. |
| 2 | **Player no navegador (Web Audio)** | Dois `<audio>` ou dois `AudioBufferSource` podem sofrer deriva; microfone via `getUserMedia` tem latência alta | Reproduzir as stems no **motor nativo**, mixando vocal + instrumental no mesmo callback de áudio (sincronia por amostra). A UI só envia comandos. |
| 3 | **Empacotar Python + PyTorch + Demucs** | Instalador pode passar de 2 GB (com CUDA); PyInstaller com torch é frágil | Não embutir os modelos nem o CUDA no instalador. Baixar o runtime/modelo no primeiro uso, ou avaliar **ONNX Runtime** para a inferência. Decidir no spike. |
| 4 | **Autotune em tempo real** | É a funcionalidade mais difícil e a de maior risco técnico (detecção de pitch + deslocamento de pitch com latência baixa) | Fazer um **spike isolado** antes do Sprint 6. Considerar começar pela correção de afinação simples (YIN + resample/PSOLA). |
| 5 | **Latência no Windows** | WASAPI em modo compartilhado costuma ficar em dezenas de ms, ruim para cantar | Planejar WASAPI exclusivo e, depois, ASIO (o SDK tem licença própria). Recomendar fones e medir cedo. |
| 6 | **Transposição** | Mudar o tom ao vivo exige algoritmo de pitch shift de qualidade | Duas estratégias: tempo real no motor (Signalsmith Stretch ou Rubber Band, conferir licenças) ou pré-renderizar e cachear por semitom. Decidir junto com o motor de áudio. |
| 7 | **Detecção de tonalidade com Librosa** | Librosa não traz detector de tom pronto; a precisão com chroma + perfis Krumhansl é limitada | Tratar como "sugestão" com ajuste manual (o plano já prevê). |
| 8 | **Demucs** | O repositório original da Meta está arquivado; há versões de torch/torchaudio que quebram o carregamento de áudio | Fixar versões, criar uma interface `Separator` e permitir trocar o modelo (ex.: modelos Roformer via `python-audio-separator`) sem mexer no resto. |
| 9 | **Armazenamento** | Stems em WAV ocupam muito disco por música | Guardar stems em **FLAC**. |
| 10 | **Segurança do backend local** | Qualquer processo local pode acessar uma porta HTTP aberta | Escutar só em `127.0.0.1`, porta aleatória, token enviado pelo Tauri ao iniciar o sidecar. |
| 11 | **`shared-types` manual** | Tipos TS e Pydantic divergem com o tempo | Gerar os tipos TS a partir do **OpenAPI do FastAPI** (`openapi-typescript`). |
| 12 | **Faltam no plano** | — | CI Windows + macOS, assinatura de código (SmartScreen / notarização da Apple), auto-update (Tauri updater), migrações do SQLite, fila de jobs com cancelamento, gerenciador de download de modelos, idioma da interface (pt-BR), checagem de espaço em disco. |

As decisões marcadas acima são recomendações. O próprio documento original diz que as escolhas devem ser validadas nos protótipos, então a **Fase 0** existe para isso.

---

## 2. Arquitetura revisada

```text
┌─────────────────────────── Tauri (processo único) ───────────────────────────┐
│  WebView: React + TS + Vite   ←── comandos/eventos ──→   Núcleo Rust          │
│  (UI, biblioteca, letras)                                  ├─ Audio Engine    │
│                                                            │   (cpal, mixer,  │
│                                                            │    efeitos, mic) │
│                                                            └─ Gerência do     │
│                                                               sidecar Python  │
└───────────────────────────────────────────────┬──────────────────────────────┘
                                                │ HTTP + WebSocket (127.0.0.1, token)
                                   ┌────────────▼─────────────┐
                                   │ Music Engine (Python)    │
                                   │ FastAPI, Demucs, FFmpeg, │
                                   │ Librosa, SQLite, LRCLIB  │
                                   └──────────────────────────┘
```

Princípios:

- **Tempo real fica em Rust**: reprodução, mixer, microfone, efeitos. Nunca depende do Python estar ocupado.
- **Trabalho demorado fica em Python**: importar, separar, analisar, buscar letras.
- **A UI nunca toca em áudio de baixo nível**: ela só envia comandos (play, seek, volume) e recebe eventos (posição, níveis, progresso).
- **Contrato único**: o OpenAPI do FastAPI gera os tipos usados pela UI.

### Estrutura do repositório (ajustada)

```text
karaoke-studio/
├── apps/
│   └── desktop/
│       ├── src/                  # React + TS
│       │   ├── features/{library,player,lyrics,mixer,settings}/
│       │   ├── hooks/ services/ components/
│       │   └── App.tsx
│       └── src-tauri/
│           ├── src/
│           │   ├── audio/        # engine: mixer, playback, mic, dsp
│           │   ├── sidecar.rs    # inicia/monitora o Python
│           │   └── main.rs
│           ├── capabilities/
│           └── tauri.conf.json
├── services/
│   └── music-engine/
│       ├── app/{api,services,processors,models,db}/
│       ├── migrations/
│       └── tests/
├── packages/
│   └── api-client/               # tipos gerados do OpenAPI
├── spikes/                       # protótipos descartáveis da Fase 0
├── docs/
│   ├── architecture.md
│   ├── development.md
│   └── decisions/                # ADRs curtos
├── .github/workflows/
└── README.md
```

Ferramentas: **pnpm** (workspaces), **uv** (Python), **Rust stable**, **FFmpeg** no PATH em desenvolvimento.

---

## 3. Plano de execução

Estimativas assumem um desenvolvedor, em ritmo parcial; servem para ordenar, não como compromisso.

### Fase 0 — Spikes e decisões (1–2 semanas)

Objetivo: reduzir os maiores riscos antes de construir.

- [ ] **Hardware:** registrar CPU, GPU (marca/modelo/VRAM) e RAM do PC de desenvolvimento.
- [ ] **Spike Demucs:** separar 3 músicas de teste na CPU e na GPU (se houver); medir tempo, uso de memória e qualidade. Testar `htdemucs` com `--two-stems=vocals`.
- [ ] **Spike áudio:** em Rust, abrir saída com `cpal` e tocar duas stems sincronizadas; medir deriva em 10 minutos.
- [ ] **Spike latência:** abrir entrada + saída do microfone, medir latência ida-e-volta com WASAPI (compartilhado e exclusivo).
- [ ] **Spike autotune:** detecção de pitch (YIN) + correção simples em arquivo gravado, ainda sem tempo real.
- [ ] **Spike empacotamento:** gerar um executável do sidecar Python (PyInstaller ou Python embutido) e medir o tamanho.
- [ ] Registrar cada decisão em `docs/decisions/` (motor de áudio, estratégia de modelos, estratégia de transposição).

**Critério de saída:** decisão documentada sobre Rust vs JUCE, forma de distribuir o PyTorch/modelos, e viabilidade de latência aceitável.

### Fase 1 — Fundação (Sprint 1)

- [ ] Criar o repositório em `C:\Users\umpie\projects\karaoke-studio`, `git init`, `.gitignore`, `README`.
- [ ] Monorepo com pnpm workspaces.
- [ ] App Tauri v2 + Vite + React + TypeScript.
- [ ] Serviço FastAPI com `/health`, rodando em `127.0.0.1` com porta aleatória e token.
- [ ] Tauri inicia e encerra o sidecar Python; UI mostra o status do backend.
- [ ] Geração de tipos TS a partir do OpenAPI.
- [ ] Lint/format (ESLint, Prettier, Ruff, rustfmt) e CI básico (Windows).
- [ ] Logs locais (arquivo rotativo) no backend e no Tauri.

**Entrega:** janela abre, backend sobe junto, a UI mostra "conectado".

### Fase 2 — Importação e biblioteca (Sprint 2)

- [ ] SQLite com migrações (esquema: músicas, jobs, configurações).
- [ ] Importar MP3, WAV, FLAC (seletor de arquivos nativo + arrastar e soltar).
- [ ] Extrair metadados (mutagen/ffprobe): título, artista, duração, formato, taxa de amostragem, canais.
- [ ] Copiar/registrar arquivos na pasta de dados do app; detectar duplicatas por hash.
- [ ] Tela de biblioteca: lista, busca por título/artista, status, remoção.
- [ ] Testes de importação com arquivos de exemplo (incluindo arquivo corrompido).

**Entrega:** biblioteca funcional e persistente.

### Fase 3 — Separação por IA (Sprint 3)

- [ ] Interface `Separator` + implementação Demucs.
- [ ] Fila de jobs com **um worker** (separação é pesada), com cancelamento e retomada após reinício.
- [ ] Progresso via WebSocket; erros claros na UI.
- [ ] Detectar dispositivo (CUDA / CPU) e informar ao usuário.
- [ ] Gerenciador de modelo: baixar no primeiro uso, verificar integridade, mostrar progresso.
- [ ] Salvar `vocals.flac` e `instrumental.flac` com a mesma duração e taxa do original.
- [ ] Teste automático: soma das stems ≈ original, duração idêntica ao sample.
- [ ] Reprocessar uma música.

**Entrega:** músicas importadas ficam prontas para karaokê.

### Fase 4 — Player e mixer (Sprint 4) — **primeiro MVP**

- [ ] Motor Rust: decodificar as stems (symphonia) e mixar em um único callback de áudio.
- [ ] Comandos: play, pause, seek, volume master, volume vocal, volume instrumental.
- [ ] Eventos de posição e nível para a UI (taxa limitada, ex. 30 Hz).
- [ ] Presets: tradicional (0%), referência (20%), cantar junto (50%).
- [ ] Seleção do dispositivo de saída.
- [ ] Teste de sincronia: reprodução longa sem deriva; seek repetido sem estalos (fade curto).
- [ ] Fila de reprodução básica e histórico.

**Entrega (marco do plano original):** importar, separar, tocar as duas faixas sincronizadas com volumes independentes.

### Fase 5 — Letras e análise (Sprint 5)

- [ ] Cliente LRCLIB: busca por artista + título + duração; cache local; tratamento offline.
- [ ] Parser LRC; exibição da linha atual, próximas linhas, rolagem suave.
- [ ] Ajuste manual de offset (± ms) salvo por música.
- [ ] Alternativa para letra sem timestamp (exibição estática).
- [ ] BPM e tonalidade com Librosa, marcados como sugestão editável.
- [ ] Transposição conforme a decisão da Fase 0 (tempo real no motor ou pré-render).
- [ ] Seletor de tonalidade na tela principal.

**Entrega:** karaokê com letra sincronizada e transposição.

### Fase 6 — Microfone e efeitos (Sprint 6)

- [ ] Captura de entrada, seleção de dispositivo, monitoramento com controle de volume.
- [ ] Cadeia de efeitos no motor: noise gate → EQ → compressor → autotune → reverb/delay.
- [ ] Autotune: tom automático/manual, escala, intensidade, velocidade de correção.
- [ ] Configurações: sample rate, tamanho do buffer, exibição da latência estimada.
- [ ] Presets de voz salvos.
- [ ] Testes de carga: sem dropouts durante separação em segundo plano.

**Entrega:** experiência completa de karaokê com voz processada.

### Fase 7 — Distribuição (Sprint 7)

- [ ] Build Windows (MSI/NSIS) com sidecar empacotado.
- [ ] Build macOS (ARM e Intel), com `Core Audio` e aceleração MPS quando disponível.
- [ ] Assinatura de código (Windows) e assinatura + notarização (macOS; exige conta Apple Developer).
- [ ] Auto-update (Tauri updater).
- [ ] CI com builds nas duas plataformas.
- [ ] Documentação de instalação, atualização e solução de problemas.

**Entrega:** instaladores para Windows e macOS.

---

## 4. Qualidade e testes

- **Python:** pytest para importação, metadados, jobs e LRC; arquivo de áudio sintético (tom/clique) para testes rápidos.
- **Rust:** testes unitários do mixer e dos efeitos com buffers sintéticos; teste de ganho/clipping.
- **UI:** Vitest + Testing Library para componentes; Playwright opcional para fluxos.
- **Áudio:** teste de deriva em reprodução longa; teste de seek em estresse.
- **Regra do tempo real:** nenhuma alocação, bloqueio ou I/O no callback de áudio.
- **CI:** lint + testes a cada push; build de release por tag.

---

## 5. Riscos principais

| Risco | Impacto | Mitigação |
|---|---|---|
| Latência do microfone alta no Windows | Cantar fica desconfortável | Spike na Fase 0; modo exclusivo/ASIO; recomendar fones |
| Autotune com artefatos | Qualidade vocal ruim | Começar simples, iterar; spike isolado |
| Instalador gigante | Distribuição ruim | Baixar modelos e runtime sob demanda |
| Separação lenta sem GPU | Frustração do usuário | Mostrar estimativa, rodar em segundo plano, cachear |
| Vazamento entre stems | Voz residual no instrumental | Avisar na UI; permitir trocar de modelo |
| Letras incompletas/ausentes | Experiência incompleta | Fallback sem timestamp e edição manual |
| Direitos autorais | Risco legal | Apenas arquivos locais do usuário; avaliar fontes e letras antes de qualquer distribuição |

---

## 6. Preparação do ambiente (Windows 11)

Pré-requisitos, instalados uma vez:

```powershell
winget install Git.Git
winget install OpenJS.NodeJS.LTS
winget install pnpm.pnpm
winget install Rustlang.Rustup
winget install astral-sh.uv
winget install Gyan.FFmpeg
# Tauri também precisa do Microsoft C++ Build Tools e do WebView2 (já incluso no Windows 11)
```

Criação do projeto:

```powershell
cd C:\Users\umpie\projects
mkdir karaoke-studio
cd karaoke-studio
git init
pnpm init
# apps/desktop: pnpm create tauri-app  (React + TypeScript)
# services/music-engine: uv init + uv add fastapi uvicorn
```

---

## 7. Decisões tomadas e perguntas em aberto

### Hardware confirmado

| Componente | Detalhe | Consequência |
|---|---|---|
| GPU | NVIDIA RTX 4060 (8 GB VRAM) | Demucs roda em **CUDA**; 8 GB bastam para `htdemucs` (se faltar memória, reduzir o tamanho do segmento). Instalar o PyTorch com build CUDA. |
| CPU | AMD Ryzen 5 7600 | Boa para FFmpeg, Librosa e DSP; serve como fallback sem GPU. |
| RAM | 32 GB | Folga para modelo + biblioteca + app. |
| Placa-mãe | B650M-E WIFI | Áudio onboard serve para começar, mas a latência do microfone deve ser medida na Fase 0. Um microfone USB ou interface de áudio com driver ASIO tende a ajudar. |

### Uso pessoal: o que muda

- **Fase 7 encolhe:** sem distribuição pública, não são necessários assinatura de código, notarização, auto-update nem build macOS. Basta um instalador local ou até rodar direto do repositório.
- **Empacotamento do Python deixa de ser problema:** o backend pode rodar a partir do ambiente `uv` (venv) com PyTorch CUDA. PyInstaller e a distribuição de modelos ficam opcionais.
- **Licenças pesam menos:** JUCE, Rubber Band e o SDK do ASIO ficam mais acessíveis em uso pessoal. Se um dia houver distribuição, isso precisa ser reavaliado.
- **macOS vira opcional**, não parte do caminho principal.

### Ainda em aberto

1. **Motor de áudio:** recomendo **Rust** (`cpal` + `symphonia`) dentro do Tauri. Se não houver objeção, é o padrão do plano; a Fase 0 confirma ou derruba com os protótipos.
2. **Microfone e fones** que você usa (modelo e tipo de conexão), para os testes de latência.
3. **Idioma da interface:** português (pt-BR), inglês ou ambos.

---

## 8. Próximo passo

Executar a **Fase 0**, começando pelo spike do Demucs e pelo spike de áudio em Rust, e então criar o repositório com a **Fase 1**.
