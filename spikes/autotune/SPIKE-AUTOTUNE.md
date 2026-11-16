# Spike 4: autotune (detecção de altura + correção)

Objetivo: saber se um autotune simples soa aceitável em voz cantada e quanto custa em CPU e atraso.
Aqui o processamento é feito em arquivo (offline), mas o código já é de streaming: processa blocos de
qualquer tamanho, sem alocar, como o callback do motor final. Um teste confirma que a saída é idêntica
com blocos de 1, 333 e 4096 amostras.

## Como funciona
1. **Detector YIN** (janela de 16 ms, atualiza a cada 5 ms) acha a altura da voz; sem voz, não corrige.
2. **Quantização**: acha a nota mais próxima da escala escolhida (com histerese, para não ficar trocando de nota na fronteira).
3. **Velocidade**: a correção é suavizada (`--speed-ms`). 0 = instantâneo ("robô"); 100+ ms preserva o vibrato.
4. **Deslocador**: linha de atraso que lê um pouco mais rápido ou mais devagar e pula exatamente um período
   da voz de vez em quando (como o PSOLA). Isso é barato e tem pouca latência.

## Rodar (copie a pasta para `spikes\autotune`)
```powershell
cd C:\Users\umpie\projects\karaoke-studio\spikes\autotune
cargo test --release

# 1) com o vocal que o Demucs separou (troque NOME pela pasta da música)
cargo run --release -- ..\demucs\out\htdemucs\NOME\vocals.wav tuned_suave.wav --speed-ms 30
cargo run --release -- ..\demucs\out\htdemucs\NOME\vocals.wav tuned_robo.wav  --speed-ms 0
```
O vocal original já é afinado, então use o `tuned_robo` para ouvir o efeito e os artefatos.

```powershell
# 2) com a sua voz: grave 20 s cantando no Gravador de Voz do Windows e converta
ffmpeg -i "Gravação.m4a" -ar 48000 -ac 1 minha_voz.wav
cargo run --release -- minha_voz.wav minha_voz_tuned.wav --speed-ms 20
cargo run --release -- minha_voz.wav minha_voz_fraco.wav --speed-ms 80 --strength 0.6
```
Sem `--key`/`--scale`, o padrão é cromático (vai para o semitom mais próximo). Com tom e escala:
`--key C --scale major` (também `minor`, `pent-major`, `pent-minor`).

## O que me enviar
- A saída completa de cada comando (inclui desvio antes/depois, atraso e CPU).
- Como soou, com fones: voz com "chiado" ou "metálica", cliques nos cortes entre notas, notas erradas
  (oitava errada, nota vizinha), respiração e consoantes ("s", "t") estranhas, e se o efeito suave soa natural.

## Limites conhecidos (a avaliar na escuta)
- A voz muda um pouco de timbre ao subir/descer (os formantes acompanham a altura). Para correções de
  até ~1 semitom é pequeno.
- Vozes com muito ruído ou reverb (como o vocal do Demucs) confundem o detector; a voz ao vivo no microfone é mais limpa.
- O detector roda no mesmo laço do áudio. No app final ele irá para outra thread para o callback ficar leve.
