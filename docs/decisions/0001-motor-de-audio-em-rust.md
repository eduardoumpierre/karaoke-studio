# 0001. Motor de áudio em Rust dentro do Tauri

Status: aceita (spikes 2 e 3, Fase 0)

**Decisão:** o motor de áudio é Rust (`cpal` + `symphonia`, e `wasapi` no Windows), dentro do processo Tauri. JUCE foi descartado.

**Motivos:** as duas stems são misturadas no mesmo callback (sincronia por construção, sem deriva); folga de CPU enorme na medição; menos uma linguagem e um processo no projeto.

**Consequências:** reamostragem de qualidade (`rubato`) no motor final; a posição publicada é a escrita no dispositivo, então as letras descontam a latência de saída medida.
