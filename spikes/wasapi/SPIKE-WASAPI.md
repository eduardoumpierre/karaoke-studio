# Spike 3b: latência com WASAPI (compartilhado, baixa latência e exclusivo)

Objetivo: descobrir se dá para ficar abaixo de ~40 ms de ida e volta (voz → mic → app → fone) no Windows.
No spike 3a, o modo compartilhado do cpal deu 71 ms (Realtek) e 105 ms (EDIFIER por USB).

**Aviso honesto:** a lógica pura (`dsp.rs`) tem 10 testes que passam, e `win.rs` compila contra um stub
com as assinaturas da crate `wasapi` 0.27, mas o código WASAPI real não pôde ser compilado nem rodado
aqui. Se der erro de compilação, cole a saída e eu corrijo.

## Preparação

1. Feche Spotify, navegador com áudio, Discord, etc. (o modo exclusivo precisa do dispositivo livre).
2. Painel de Som → aba Reprodução → fone Realtek → Propriedades → Avançado → marque
   **"Permitir que aplicativos assumam o controle exclusivo deste dispositivo"**. Repita em Gravação
   para o microfone Realtek.
3. Monte como no 3a: o microfone do headset perto do fone (encoste o fone no mic, ou use o fone na orelha
   com o mic próximo). **Cuidado:** o chirp sai nos fones. Comece com `--level 0.2`.

## Rodar (na pasta `spike-wasapi`)

```powershell
cargo test
cargo run --release -- --list

# 0) controle: nivel 0 deve rejeitar tudo (sem sinal)
cargo run --release -- --mode shared --output Realtek --input Realtek --level 0 --clicks 5

# 1) compartilhado
cargo run --release -- --mode shared    --output Realtek --input Realtek --level 0.3
# 2) compartilhado de baixa latencia (IAudioClient3)
cargo run --release -- --mode lowlat    --output Realtek --input Realtek --level 0.3
# 3) exclusivo (periodo minimo do driver)
cargo run --release -- --mode exclusive --output Realtek --input Realtek --level 0.3
# 4) exclusivo com periodo maior, se o minimo falhar
cargo run --release -- --mode exclusive --output Realtek --input Realtek --period-ms 5 --level 0.3
```

Fique em silêncio durante os testes. Rode também o modo exclusivo com `--output EDIFIER`
(a EDIFIER por USB provavelmente não ajuda, mas vale medir).

## O que me enviar

A saída completa de cada comando (formato, período, buffer, latência reportada, resumo).

## Como interpretar

| Ida e volta | Sensação ao cantar |
|---|---|
| ≤ 20 ms | imperceptível |
| 20–40 ms | bom |
| 40–60 ms | no limite |
| > 60 ms | difícil |

Erros comuns do exclusivo: "negado ou em uso" (passo 2 da preparação, ou outro app usando o dispositivo)
e "formato não suportado" (o spike já testa 24/32/16 bits, 48 e 44,1 kHz, 1 e 2 canais).
Cada metro entre fone e mic soma ~3 ms.
