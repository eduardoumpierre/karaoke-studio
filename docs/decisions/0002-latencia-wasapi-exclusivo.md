# 0002. Latência de monitoramento: WASAPI exclusivo

Status: aceita (spikes 3a e 3b)

**Medido (ida e volta, Realtek):** compartilhado 71 ms; IAudioClient3 71 ms (driver só oferece 10 ms); **exclusivo 4,7 ms** (período 3 ms). EDIFIER USB em exclusivo: 66 ms (atraso interno da placa USB; o valor reportado pelo driver, 4,67 ms, é falso).

**Decisão:** no Windows o caminho principal é WASAPI exclusivo (`wasapi`), com `cpal` compartilhado como alternativa. Música e monitor da voz saem pelo mesmo dispositivo (fones na Realtek). A latência alvo é configurável (3 a 10 ms). O app mede a latência de cada dispositivo com o chirp; nunca confia no valor reportado.

**Consequências:** o exclusivo bloqueia o dispositivo para outros apps; o formato do dispositivo (24 bits, 48 kHz) exige conversão e reamostragem; caixa USB serve só para a música (letras descontam a latência medida). Melhor opção para festa: ligar a caixa por cabo analógico (AUX), a testar quando houver cabo.
