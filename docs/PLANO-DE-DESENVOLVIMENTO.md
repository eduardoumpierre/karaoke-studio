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

- [x] **Hardware:** registrar CPU, GPU (marca/modelo/VRAM) e RAM do PC de desenvolvimento.
- [x] **Spike Demucs:** separar 3 músicas de teste na CPU e na GPU (se houver); medir tempo, uso de memória e qualidade. Testar `htdemucs` com `--two-stems=vocals`.
- [x] **Spike áudio:** em Rust, abrir saída com `cpal` e tocar duas stems sincronizadas; medir deriva em 10 minutos.
- [x] **Spike latência:** abrir entrada + saída do microfone, medir latência ida-e-volta com WASAPI (compartilhado e exclusivo).
- [x] **Spike autotune:** detecção de pitch (YIN) + correção simples em arquivo gravado, ainda sem tempo real.
- [ ] *(adiado, uso pessoal)* **Spike empacotamento:** gerar um executável do sidecar Python (PyInstaller ou Python embutido) e medir o tamanho.
- [ ] Registrar cada decisão em `docs/decisions/` (motor de áudio, estratégia de modelos, estratégia de transposição).

**Critério de saída:** decisão documentada sobre Rust vs JUCE, forma de distribuir o PyTorch/modelos, e viabilidade de latência aceitável.

### Fase 1 — Fundação (Sprint 1)

- [x] Criar o repositório em `C:\Users\umpie\projects\karaoke-studio`, `git init`, `.gitignore`, `README`.
- [x] Monorepo com pnpm workspaces.
- [x] App Tauri v2 + Vite + React + TypeScript.
- [x] Serviço FastAPI com `/health`, rodando em `127.0.0.1` com porta aleatória e token.
- [x] Tauri inicia e encerra o sidecar Python; UI mostra o status do backend.
- [x] Geração de tipos TS a partir do OpenAPI.
- [x] Lint/format (ESLint, Prettier, Ruff, rustfmt) e CI básico (Windows). *(o `ci.yml` está pronto; falta copiar para `.github/workflows`, pois a ferramenta não grava nessa pasta)*
- [x] Logs locais (arquivo rotativo) no backend e no Tauri.

**Entrega:** janela abre, backend sobe junto, a UI mostra "conectado". **Concluída em 13/11/2026:** `pnpm tauri dev` abriu a janela e o sidecar subiu na porta escolhida (log: "backend pronto na porta 50565").

Notas da fase: o pnpm do PC é a versão 11 (política de idade mínima de pacotes: o lockfile só usa versões com mais de 2 dias; `allowBuilds` para o esbuild); o Vite precisa ignorar `src-tauri` (senão trava no Windows).

### Fase 2 — Importação e biblioteca (Sprint 2)

- [x] SQLite com migrações (esquema: músicas, jobs, configurações).
- [x] Importar MP3, WAV, FLAC (seletor de arquivos nativo + arrastar e soltar). *(backend testado; seletor e arrastar dependem de teste na janela do Tauri)*
- [x] Extrair metadados (mutagen/ffprobe): título, artista, duração, formato, taxa de amostragem, canais.
- [x] Copiar/registrar arquivos na pasta de dados do app; detectar duplicatas por hash.
- [x] Tela de biblioteca: lista, busca por título/artista, status, remoção.
- [x] Testes de importação com arquivos de exemplo (incluindo arquivo corrompido).

**Entrega:** biblioteca funcional e persistente.

Notas: a API é `GET /songs?q=`, `GET /songs/{id}`, `POST /songs/import` (caminho do arquivo) e `DELETE /songs/{id}`; erros com código (`duplicate`, `invalid_audio`, `unsupported_format`, `file_not_found`, `tool_unavailable`). A busca ignora maiúsculas e acentos. A normalização para WAV 44,1 kHz (ADR 0003) acontece no início do job de separação (Fase 3), não na importação. Um teste na tela com navegador real achou um bug de threads do SQLite que os testes de API não pegavam (corrigido e coberto por teste de concorrência).

### Fase 3 — Separação por IA (Sprint 3)

- [x] Interface `Separator` + implementação Demucs (processo separado: cancelável e devolve a VRAM ao terminar).
- [x] Fila de jobs com **um worker**, com cancelamento (na fila e em execução). *Retomada após reinício:* jobs que estavam rodando viram "falhos (interrompido)" e os que estavam na fila continuam sozinhos.
- [x] Progresso na UI (consulta a cada 1 s enquanto há job ativo, no lugar do WebSocket planejado: mais simples e suficiente); erros claros na UI com "Tentar de novo".
- [ ] Detectar dispositivo (CUDA / CPU) e informar ao usuário. *(hoje o Demucs escolhe sozinho; `KARAOKE_DEVICE` força cuda/cpu/mps)*
- [ ] Gerenciador de modelo: baixar no primeiro uso com progresso e verificar integridade. *(hoje baixa sozinho na primeira separação, para `%LOCALAPPDATA%\dev.karaokestudio.app\models`, sem progresso próprio)*
- [x] Salvar `vocals.flac` e `instrumental.flac` (24 bits) em `songs/<id>/stems/`, a partir do WAV 44,1 kHz estéreo normalizado.
- [ ] Teste automático com Demucs real: soma das stems ≈ original e mesma duração (`tests/test_demucs_real.py`, roda só com `uv sync --group ml`; **ainda não executado**).
- [x] Reprocessar uma música ("Refazer").

**Entrega:** músicas importadas ficam prontas para karaokê.

Notas: API `POST /songs/{id}/separate?force=`, `GET /jobs?active=`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel`. Não dá para remover uma música com job ativo (erro `song_busy`). O Demucs real **não foi executado no ambiente de desenvolvimento**: o leitor de progresso e o cancelamento foram testados com um programa falso que imita a saída do tqdm; a primeira separação de verdade no seu PC é o teste que falta.

### Fase 4 — Player e mixer (Sprint 4) — **primeiro MVP**

- [x] Motor Rust (`apps/desktop/src-tauri/audio-engine`, sem dependência do Tauri): decodifica as stems com symphonia (FLAC, MP3, WAV), reamostra uma vez ao carregar (rubato) e mixa as duas faixas num único callback de áudio (cpal).
- [x] Comandos: tocar, pausar, seek, volume geral, volume da voz e do instrumental (suavizados, sem estalo).
- [x] Eventos de posição e nível para a UI a ~30 Hz (`player-event`).
- [x] Presets: Tradicional (voz 0%), Referência (20%), Cantar junto (50%).
- [x] Seleção do dispositivo de saída (a música pode sair na caixa de som) e de entrada (microfone do headset), salvas em `audio-settings.json`; teste de nível do microfone. *A entrada só é usada para o microfone ao cantar na Fase 6.*
- [x] Teste de sincronia (lógica): 10 minutos renderizados em blocos de tamanhos variados, com posição exata e amostras idênticas à fonte; 40 seeks seguidos sem estalo; pausa/retomada e troca de música com fade de 8 ms.
- [x] Fila de reprodução básica e histórico (histórico e volumes ficam salvos).
- [ ] **Ainda não validado no PC:** a ligação com o Tauri (`player.rs`) não compilou aqui (o sandbox não tem as bibliotecas do WebView); o motor foi testado só em lógica, sem placa de som. Falta ouvir de verdade e checar sincronia voz × instrumental.
- [ ] Compensar a latência da saída na posição mostrada (importante para as letras da Fase 5).

**Entrega (marco do plano original):** importar, separar, tocar as duas faixas sincronizadas com volumes independentes.

Notas: as músicas ficam na memória decodificadas (≈ 45 MB por minuto por faixa a 48 kHz): uma música de 5 min usa ~230 MB com as duas faixas. Ao trocar de dispositivo de saída, a música é recarregada na nova taxa e volta ao ponto onde estava. Endpoint novo: `GET /songs/{id}/audio` (caminhos dos arquivos).

### Fase 5 — Letras e análise (Sprint 5)

- [x] Cliente LRCLIB: busca por artista + título + duração; cache local; tratamento offline. *(testado só com respostas simuladas; a rede do ambiente de desenvolvimento não alcança o LRCLIB — validar na sua máquina.)*
- [x] Parser LRC; exibição da linha atual, próximas linhas, rolagem suave.
- [x] Ajuste manual de offset (± ms) salvo por música.
- [x] Alternativa para letra sem timestamp (exibição estática) e letra colada manualmente.
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

### Decisões fechadas

- **Motor de áudio:** Rust (`cpal` + `symphonia`) dentro do Tauri. A Fase 0 confirma ou derruba com os protótipos.
- **Idioma da interface:** português (pt-BR). Textos em um único arquivo de strings desde o início, para facilitar outros idiomas no futuro.
- **Áudio:** fones Sennheiser em P2 na placa-mãe (áudio onboard, tipicamente Realtek). Na Fase 0, medir a latência em WASAPI compartilhado e exclusivo; se ficar alta, avaliar driver ASIO ou uma interface/microfone USB.

### Ainda em aberto

1. **Qual microfone será usado para cantar?** Os fones têm microfone embutido, ou há um microfone separado (P2 ou USB)? Isso define o teste de latência e o ruído de captura.

---

## 8. Resultados da Fase 0

### Spike Demucs (RTX 4060, `htdemucs`, `--two-stems=vocals`)

- **Ambiente:** PyTorch 2.5.1+cu124, CUDA ativo, Python 3.11 (o PyTorch 2.5.1 não tem build para Python 3.13). Esse ambiente está fixado no `pyproject.toml` do spike.
- **Desempenho (1 música, 278 s):** 20,4 s no total, 0,07x a duração, incluindo o download do modelo (84 MB). A barra do Demucs indicou cerca de 9 s só para separar. O número definitivo vem da segunda execução, com o modelo em cache.
- **Sincronia:** vocals e instrumental saem com exatamente o mesmo número de amostras.
- **Descoberta:** o Demucs trabalha a **44,1 kHz** e reamostra a entrada (o original estava a 48 kHz). A duração das stems ficou 0,03 s maior que a do original.

**Decisões decorrentes**

1. As stems da biblioteca ficam em **44,1 kHz**, e o player toca **só as stems** (a faixa original não é necessária no mixer: "voz de referência" é apenas o volume do vocal).
2. O motor de áudio faz a conversão para a taxa do dispositivo de saída (por exemplo, 48 kHz) em um único ponto, para vocal e instrumental ficarem sempre juntos.
3. O offset das letras sincronizadas (LRC) é medido em relação ao tempo da música, e a diferença de 0,03 s é tratada como atraso fixo no ajuste manual.
4. Teste automático da Fase 3: as stems devem ter o mesmo número de amostras, e a soma deve reconstruir o original (reamostrado) com resíduo baixo após alinhar.

**Resultados completos (música de 278 s, modelo em cache)**

| Teste | Resultado |
|---|---|
| GPU (RTX 4060) | 11,4 s (0,04x a duração) |
| CPU (Ryzen 5 7600) | 75,6 s (0,27x a duração) |
| MP3 original (48 kHz) | stems atrasadas **1015 amostras (23,02 ms)** em relação ao original |
| WAV a 44,1 kHz (mesma música) | **atraso 0**, mesmo número de amostras do original (12.276.225), resíduo de reconstrução −26,6 dB |

**Causa do deslocamento:** o MP3 declara um início de stream em 0,023021 s (o `ffprobe` mostra `start: 0.023021`), que é exatamente os 23,02 ms medidos. A leitura do MP3 pelo Demucs preserva esse início como silêncio no começo; uma conversão simples pelo ffmpeg descarta. Não é um defeito do separador, e com WAV o alinhamento é perfeito.

**Resíduo de −26,6 dB:** é a parte do sinal que o modelo não atribui a nenhuma das duas stems, comum em separadores neurais. Não indica problema de sincronia. O limite de −30 dB usado no script era arbitrário.

**Decisão: normalizar na importação.** O pipeline de importação converte toda música, uma única vez, para um WAV de referência a **44,1 kHz estéreo** (via ffmpeg, sem o silêncio de início do MP3), e o Demucs roda sobre esse arquivo. Assim:

1. as stems têm exatamente o mesmo número de amostras do arquivo de referência e atraso zero;
2. o tempo da música tem uma única definição, a mesma usada pelo player e pelas letras sincronizadas;
3. o teste automático da Fase 3 passa a exigir: mesma contagem de amostras e atraso 0 entre referência e stems.

**Conclusão do spike (encerrado):**

- **Atraso constante:** medido em 5%, 50% e 90% da música, o atraso do MP3 foi de 1015 amostras nos três pontos (variação de 0 amostras entre início e fim). Não há deriva.
- **Qualidade:** avaliação de ouvido do usuário: separação de voz e fundo "quase perfeita" com `htdemucs`. O modelo atende; a interface `Separator` continua prevista para trocar de modelo no futuro, mas não é urgente.
- **Decisão confirmada:** `htdemucs` com `--two-stems=vocals` na GPU (cerca de 0,04x a duração) e fallback na CPU (cerca de 0,27x).
- **Ambiente fixado:** Python 3.11, PyTorch 2.5.1+cu124, `demucs`, `soundfile`, `numpy` (ver `pyproject.toml` do spike).

---

### Spike de áudio em Rust (`cpal` 0.15, WASAPI compartilhado, Windows 11)

| Teste | Resultado |
|---|---|
| Testes automáticos no Windows | 5 de 5 passam |
| Dispositivos | Saídas: EDIFIER R19BT (padrão) e Realtek(R) Audio, ambas 48 kHz, 2 canais, F32. Entrada: Microfone (Realtek). |
| Sincronia vocal/instrumental (ouvido) | Boa |
| Uso do callback | no máximo 0,03 ms de um buffer de 10 ms (menos de 1%) |
| Estresse de seek (60 s) | 199 seeks aleatórios sem travar nem ficar mudo |
| Deriva do dispositivo vs relógio (60 s) | oscila entre cerca de −9 e +6 ms, sem tendência clara; a medição tem resolução de cerca de 10 ms |
| `--buffer 128` e `--buffer 256` | sem efeito: os callbacks continuaram com 480 amostras (10 ms) |

**Conclusões**

1. **Rust + `cpal` está confirmado para a reprodução.** As duas stems são lidas pela mesma posição no mesmo callback, então não derivam entre si por construção, e a folga de CPU é enorme.
2. **A deriva de 10 minutos não foi medida** (só cerca de 1 minuto). Ela não afeta a sincronia das stems nem das letras, que seguem a posição publicada pelo callback; é só um dado sobre o relógio da placa. Rodar uma vez, quando for conveniente.
3. **Latência é o ponto aberto.** Em modo compartilhado o período do Windows ficou fixo em 10 ms, e o pedido de buffer menor foi ignorado (o `cpal` anunciou a faixa 0 a 4.294.967.295, que é um valor sem significado). Para cantar com latência baixa, o spike de latência precisa avaliar: WASAPI exclusivo, IAudioClient3 (período menor no modo compartilhado, depende do driver) e ASIO, via o crate `wasapi` ou bindings `windows`. O `cpal` 0.15 sozinho provavelmente não basta para a entrada do microfone.
4. **Posição das letras:** a posição publicada é a que foi escrita no dispositivo, não a que já saiu nos fones. O player deve descontar a latência de saída medida (e, com Bluetooth, ela é grande).
5. **Reamostragem:** o spike usa interpolação linear, que atenua levemente os agudos. Em teste (A/B abaixo) se ela explica o som "às vezes abafado" relatado. O motor final usará um reamostrador de qualidade (por exemplo `rubato`) de qualquer forma.
6. **Dispositivo:** a saída padrão do PC é o EDIFIER R19BT, não os fones Sennheiser. Para cantar, usar os fones no P2 da Realtek e confirmar que não é Bluetooth (Bluetooth costuma passar de 100 ms de atraso, inviável para voz).

---

**Som abafado:** com as stems convertidas para 48 kHz (sem reamostragem no spike), o usuário não notou o problema. A reamostragem linear era a causa; o motor final usará um reamostrador de qualidade (`rubato`).

### Spike de latência 3a (`cpal`, WASAPI compartilhado)

Método: chirp de 1 a 8 kHz emitido pela saída, gravação de 1 s da entrada e correlação cruzada normalizada (ncc) para achar o chirp. A primeira versão, por limiar, foi descartada: disparou com fala e ruído (valores de 0,4 ms a 385 ms). **Controle com a saída muda: 10 de 10 medidas rejeitadas** (ncc de 0,06 a 0,12), então a medição é confiável.

| Caminho | Ida e volta (mediana) | Variação | ncc |
|---|---|---|---|
| Saída EDIFIER R19BT → microfone do fone (Realtek) | **104,8 ms** | ±0,4 ms | 0,79 |
| Saída Realtek (concha do fone encostada no microfone) → microfone Realtek | **71,0 ms** | ±0,3 ms | 0,65 |

Conclusões:

1. Os callbacks têm 480 amostras (10 ms) na saída e na entrada, nos dois casos.
2. **Mesmo no melhor caso (Realtek), a ida e volta é de 71 ms**, acima de 60 ms, o limite prático para cantar ouvindo a própria voz. O modo compartilhado com `cpal` não basta para monitoramento ao vivo. O spike 3b é necessário.
3. A caixa EDIFIER soma cerca de **34 ms** a mais que a Realtek (a diferença inclui poucos ms de distância acústica). Para monitorar a voz, usar fones em dispositivo de baixa latência; as caixas servem para tocar a música.
4. Os 71 ms são bem mais do que dois buffers de 10 ms explicam. A origem é desconhecida; hipóteses a testar no 3b: efeitos de áudio do driver Realtek e buffers do motor de áudio do Windows.
5. A EDIFIER está ligada por **USB**: ela funciona como uma placa de som USB própria, com buffers próprios, o que é coerente com os cerca de 34 ms extras. Não é Bluetooth.

---

### Spike de latência 3b (`wasapi`, Realtek: saída Alto-falantes + Microfone)

Mesma medição do 3a (chirp + ncc; controle com nível 0: 5 de 5 rejeitadas, ncc ≤ 0,07). 20 de 20 medidas aceitas em cada modo.

| Modo | Período | Ida e volta (mediana) | Variação |
|---|---|---|---|
| Compartilhado | 10 ms (buffer 22 ms) | **71,0 ms** | ±0,2 ms |
| Compartilhado de baixa latência (IAudioClient3) | 10 ms (mín = padrão = máx = 480 amostras) | **70,9 ms** | ±0,2 ms |
| **Exclusivo, período mínimo** | **3 ms** (144 amostras) | **4,7 ms** | ±0,1 ms |
| Exclusivo, período de 5 ms | 5 ms (240 amostras) | **6,6 ms** | ±0,2 ms |

Conclusões:

1. **O modo exclusivo resolve a latência:** 4,7 ms de ida e volta, muito abaixo do limite imperceptível (20 ms). Mesmo com margem para o processamento do motor (efeitos, autotune), há folga de sobra.
2. **IAudioClient3 não ajuda neste driver:** o Realtek só oferece o período de 10 ms no modo compartilhado. Os 71 ms vêm do motor de áudio do Windows e do driver no modo compartilhado, não do período.
3. O período de 3 ms é o mínimo do dispositivo e funcionou (formato aceito: 32 bits inteiro com 24 válidos, 48 kHz, 2 canais). Sem falhas de abertura nem erros de buffer. Ainda não foi testada a estabilidade sob carga (underruns com o motor completo rodando); por isso a latência alvo do app deve ser configurável (3 a 10 ms).
4. **Consequências de arquitetura:**
   - O motor de áudio no Windows passa a usar WASAPI exclusivo (crate `wasapi`) como caminho principal, com `cpal` compartilhado como alternativa quando o exclusivo não estiver disponível ou for negado.
   - Em exclusivo, **o app é dono do dispositivo**: outros apps ficam sem som nele. Música e voz devem sair pelo mesmo dispositivo (um stream de saída que mistura instrumental, vocal guia e monitor da voz), o que combina com cantar de fone.
   - A entrada e a saída são dois streams (com relógios possivelmente distintos no mesmo codec Realtek, mas aqui o resultado foi estável); o monitor da voz passa por um buffer entre as duas threads.
   - O formato do dispositivo (24 bits, 48 kHz) exige conversão e reamostragem de 44,1 kHz para 48 kHz no motor (`rubato`).
   - Os 4,7 ms medidos incluem a posição acústica fone-microfone encostados; é o piso real do caminho.
5. **EDIFIER R19BT (USB) em exclusivo: 66,1 ms** (±0,4; período 4,67 ms, 16 bits, 48 kHz; entrada Realtek exclusiva 5 ms). O driver reporta 4,67 ms, mas o atraso real é cerca de 60 ms maior: o exclusivo não reduz a latência interna da placa USB da caixa. **A latência reportada pelo driver não é confiável; só a medição vale.** Conclusão: a EDIFIER serve para tocar a música (a posição das letras deve descontar a latência medida), mas não para monitorar a voz.
6. **Configuração recomendada para cantar:** tudo (música e monitor da voz) pelos fones na Realtek em exclusivo, 4,7 ms. Música nas caixas e voz nos fones exige dois dispositivos com latências diferentes e faz o microfone captar o som das caixas; deixar como modo avançado, não como padrão. O app deve medir a latência de cada dispositivo (calibração com o chirp) em vez de confiar no valor reportado.
7. **Não medido:** o Mac (Core Audio).

---

### Spike 4: autotune (YIN + quantização + deslocador por salto de período, offline)

Código em streaming (blocos de qualquer tamanho, sem alocação; teste confirma saída idêntica com blocos de 1, 333 e 4096 amostras). 11 testes passam no Windows.

| Entrada | Config. | Voz detectada | Desvio até a nota (mediana) antes → depois | CPU (1 núcleo) | Pior bloco |
|---|---|---|---|---|---|
| Vocal Demucs, 224 s | velocidade 30 ms | 56% | 24,2 → 18,4 cents | 3,6% | 0,40 ms |
| Vocal Demucs, 224 s | velocidade 0 (robô) | 56% | 24,2 → **7,2** cents | 3,6% | 0,68 ms |
| Voz do usuário, 19 s | velocidade 20 ms | 68% | 25,7 → **13,3** cents | 5,4% | 0,36 ms |
| Voz do usuário, 19 s | velocidade 80 ms, força 0,6 | 68% | 25,7 → 22,7 cents | 5,4% | 2,36 ms |

Conclusões:

1. **Viável em tempo real.** Custo de 1 a 6% de um núcleo; atraso do caminho do áudio de 9 a 15 ms (médio), entre 2 e 25 ms. O monitor do cantor vai **sem autotune** (fone, 4,7 ms); a voz corrigida vai só para as caixas, onde o atraso do autotune se soma aos ~60 ms da caixa.
2. **Velocidade e força funcionam como esperado:** velocidade 0 dá correção quase total (efeito "robô"); velocidades maiores preservam glides e vibrato e corrigem menos. Valores de partida para presets: natural = 80 ms e força 0,6; padrão = 20 ms; robô = 0 ms.
3. **O desvio "antes" de ~25 cents é o valor de uma altura aleatória** (distribuição uniforme até o semitom), então não prova que a voz estivesse desafinada. A métrica útil é o "depois".
4. Uma gravação baixa (pico 0,09) deu só 11% de voz detectada; com pico 0,21, 68%. O portão de volume do detector passou de 0,002 para 0,0005 (RMS). O motor final deve normalizar a entrada antes do detector (ganho automático ou portão relativo).
5. **Um bloco levou 2,36 ms** (de 2,67 ms) numa execução comum do Windows, provavelmente por agendamento da thread. Para o exclusivo de 3 ms, no motor final a detecção YIN roda em **outra thread** e o callback só aplica o deslocador; a thread do callback usa prioridade alta (MMCSS).
6. **Pendente:** a avaliação subjetiva da voz com fones (chiado, cliques, consoantes). Será refeita quando o autotune estiver no app, em tempo real.

### Fechamento da Fase 0

- Rust + `cpal`/`wasapi` no motor, Python no sidecar: **decidido**.
- Latência: **WASAPI exclusivo** resolve (4,7 ms na Realtek); caixa USB não serve para monitorar a voz.
- Demucs `htdemucs` na RTX 4060 com normalização para 44,1 kHz na importação: **decidido**.
- Autotune: **viável**, detecção em outra thread, monitor do cantor sem autotune.
- **Spike de empacotamento: adiado.** O uso é pessoal e o app roda a partir do repositório; só volta se a distribuição voltar a ser objetivo (Fase 7).
- Falta só registrar as decisões em `docs/decisions/` (arquivos entregues junto com este plano).

---

## 10. Próximo passo

**Fase 5a (letras) em validação:** abrir a aba "Letra" com uma música tocando (precisa de internet na primeira busca). Depois seguem a Fase 5b (BPM e tom com Librosa) e 5c (transposição, que exige decisão técnica: tempo real no motor ou pré-render).
