# Spike 3a (v2) — Latência de ida e volta (cpal, WASAPI compartilhado)

Objetivo: medir quanto tempo o som leva para sair do app, passar pelo Windows e pela placa, chegar ao microfone e voltar ao app. É a linha de base para decidir se o monitoramento da voz ao vivo é confortável no modo compartilhado, ou se precisamos do modo exclusivo (spike 3b).

**O que mudou da primeira versão:** o detector por limiar foi trocado por correlação cruzada com um sinal conhecido. O programa emite uma varredura (chirp de 1 a 8 kHz, 50 ms), grava 1 s da entrada e procura essa varredura na gravação. Cada medida vem com uma confiança (`ncc`, de 0 a 1); fala, teclado e ruído dão valores baixos e a medida é rejeitada, em vez de virar uma latência falsa. A primeira versão tinha disparado com ruído: apareceram valores de 7 ms (impossível, os buffers já têm 10 ms cada), 0,4 ms e até 385 ms.

Já verificado antes de você receber: compila e 6 testes automáticos passam (formato do chirp, recuperação de um atraso conhecido no meio de ruído, rejeição de ruído puro, emissão e gravação). A medição de verdade só acontece no seu PC.

## Durante o teste

- **Fique em silêncio e sem mexer no teclado e no mouse** durante cada rodada (cerca de 25 s). A versão nova tolera mais ruído, mas silêncio dá a melhor medida.
- Coloque o microfone do fone **bem perto da caixa EDIFIER** (menos de 10 cm da grade), apontado para ela. Cada metro de distância soma cerca de 3 ms ao tempo medido.
- Deixe o volume da caixa moderado a alto; o chirp dura só 50 ms.
- Confira em Configurações → Privacidade e segurança → Microfone se aplicativos para desktop podem usar o microfone.

## Preparação

Substitua o `main.rs` da pasta do spike (o `Cargo.toml` não mudou):

```text
C:\Users\umpie\projects\karaoke-studio\spikes\latency\Cargo.toml
C:\Users\umpie\projects\karaoke-studio\spikes\latency\src\main.rs
```

## Teste

```powershell
cd C:\Users\umpie\projects\karaoke-studio\spikes\latency

cargo test --release
```

**1. Controle (importante).** Com `--level 0` a saída fica muda, então **toda** medida deve ser rejeitada. Se alguma for aceita, a medição não é confiável e eu preciso saber:

```powershell
cargo run --release -- --level 0 --clicks 10
```

**2. Medição, saída EDIFIER (padrão) com o microfone do fone:**

```powershell
cargo run --release
```

**3. Opcional, saída Realtek (fones no P2 verde, microfone do fone no rosa).** Só faz sentido com o fone vestido e silêncio total; o microfone do fone fica longe das conchas, então talvez a confiança seja baixa. Se rejeitar tudo, não é problema do programa:

```powershell
cargo run --release -- --output Realtek --level 0.8
```

Se muitas medidas forem rejeitadas na rodada 2: aproxime mais o microfone, aumente o volume da caixa ou use `--level 0.8`. Para aceitar medidas menos confiáveis: `--min-ncc 0.2` (não recomendo abaixo disso).

## Como interpretar (regra prática)

| Ida e volta | Sensação ao cantar com monitoramento |
|---|---|
| até cerca de 20 ms | quase imperceptível |
| 20 a 40 ms | notável, mas confortável para a maioria |
| 40 a 60 ms | começa a atrapalhar o ritmo |
| acima de 60 ms | difícil de cantar ouvindo a própria voz |

Se o resultado ficar acima de 40 ms, o próximo passo é o spike 3b (WASAPI exclusivo e IAudioClient3). Lembre que a caixa EDIFIER pode somar latência própria (USB ou Bluetooth), por isso a rodada com saída Realtek serve de comparação.

## O que me enviar

1. A saída de `cargo test --release` (a última linha).
2. A saída completa do controle (rodada 1) e da medição (rodada 2); da rodada 3 se fizer.
3. Como as caixas EDIFIER estão ligadas ao PC: P2 na placa-mãe, USB ou Bluetooth (o Windows as lista separadas da Realtek, o que sugere USB ou Bluetooth).
