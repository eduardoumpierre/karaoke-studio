# Spike 2 — Áudio em Rust (duas stems sincronizadas)

Objetivo: confirmar que o motor de áudio em Rust (`cpal`) toca vocal e instrumental juntos, sem deriva, com volumes independentes, pausa e seek, e medir a folga do callback no seu PC.

O que este spike **testa**: sincronia vocal/instrumental, diferença entre o relógio da placa de som e o do sistema, tempo gasto no callback, seek e pausa sem estalos.

O que ele **não testa** (vem no spike 3): latência do microfone, modo exclusivo do WASAPI.

## Como funciona

As duas stems são lidas pela mesma posição, dentro do mesmo callback de áudio, então não existe deriva entre elas por construção. A posição publicada pelo callback é a que o player e as letras usarão no app. A reamostragem de 44,1 kHz para a taxa da placa (provavelmente 48 kHz) é linear neste spike; o motor final usará um reamostrador de qualidade.

Já verificado antes de você receber: o código compila e 5 testes automáticos passam (mixagem com volumes independentes, contagem de posição com reamostragem, seek com fade-in, fim e repetição, leitura de WAV float e PCM 16 bits). Não foi possível testar som de verdade, porque não há placa de áudio onde eu rodo o código.

## Preparação (uma vez)

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Feche e abra o PowerShell, depois confira:

```powershell
rustc --version
cargo --version
```

Coloque os arquivos assim:

```text
C:\Users\umpie\projects\karaoke-studio\spikes\audio-rust\Cargo.toml
C:\Users\umpie\projects\karaoke-studio\spikes\audio-rust\src\main.rs
```

(crie a pasta `src` dentro de `audio-rust`). Na primeira compilação o Cargo baixa as dependências, o que leva alguns minutos.

## Teste

```powershell
cd C:\Users\umpie\projects\karaoke-studio\spikes\audio-rust

cargo test
cargo run --release -- --list
```

`--list` mostra seus dispositivos de saída e entrada. Me envie essa lista, ela será usada no spike do microfone.

Defina os caminhos das stems (as do spike anterior):

```powershell
$v = "C:\Users\umpie\projects\karaoke-studio\spikes\demucs\out\htdemucs\last-christmas-test\vocals.wav"
$i = "C:\Users\umpie\projects\karaoke-studio\spikes\demucs\out\htdemucs\last-christmas-test\no_vocals.wav"
```

### 1. Sincronia, de ouvido

```powershell
cargo run --release -- $v $i
```

A música começa só com o instrumental. Use fones e digite os comandos no terminal, seguidos de Enter:

- `v 1` liga o vocal. Se as stems estiverem alinhadas, a música soa como o original, sem eco nem "arrastado". Um desalinhamento de 20 ms já seria audível.
- `i 0` e depois `v 1`: ouve só o vocal. `i 1` volta.
- `p` pausa e retoma; `seek 60` pula para 1 minuto. Não deve haver estalos fortes (um clique leve ao pular é esperado, pois só há fade-in neste spike).
- `q` sai.

### 2. Deriva em 10 minutos

```powershell
cargo run --release -- $v $i --vocal 1 --loop --duration 600
```

Deixe rodar, sem mexer. A cada 5 segundos aparece uma linha com a deriva do dispositivo em relação ao relógio. Valores oscilam alguns ms por causa do tamanho do buffer; o que importa é a tendência ao longo dos 10 minutos.

### 3. Estresse de seek

```powershell
cargo run --release -- $v $i --vocal 1 --seek-stress --duration 60
```

Um seek aleatório a cada 300 ms durante 1 minuto. Não deve travar, estourar nem ficar mudo.

### 4. Buffers pequenos

```powershell
cargo run --release -- $v $i --buffer 256 --duration 30
cargo run --release -- $v $i --buffer 128 --duration 30
```

Mostra se a placa aceita buffers menores e quanto do tempo do buffer o callback usa. Se a placa aceitar valor menor que o padrão, a linha "Buffer solicitado" mostra o valor aplicado.

## O que me enviar

1. A saída de `cargo test` (a última linha basta) e a de `--list`.
2. A linha "Host / Saída / Hz / canais" e o "Resumo" de cada teste (2, 3 e 4).
3. Sua avaliação de ouvido no teste 1: alinhado? estalos? ruído? Os fones Sennheiser estão ligados na saída onboard, certo?
