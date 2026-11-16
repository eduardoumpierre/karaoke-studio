# ADR 0001 — Transposição por pré-render

**Status:** aceita (Fase 5c).

## Contexto
O karaokê precisa mudar o tom da música para caber na voz de quem canta. Havia duas opções:
processar em tempo real dentro do motor de áudio ou gerar antes os arquivos no novo tom.

## Decisão
Pré-render: ao escolher um tom, o backend gera os stems (voz e instrumental) no novo tom com o
filtro `rubberband` do ffmpeg e o player apenas toca os arquivos novos, na mesma posição.

- Faixa de ±6 semitons; a voz usa `formant=preserved` para manter o timbre.
- Arquivos em `songs/<hash>/pitch/<+N>/` (FLAC 24 bits); servem de cache e são refeitos se os stems
  forem gerados de novo. Há botão para limpar o cache.
- A escolha de tom fica salva por música (`song_pitch`).
- O controle usa espera de 600 ms: cliques seguidos geram um único render.

## Consequências
- (+) Qualidade melhor (algoritmo offline) e nenhum custo de CPU extra no callback de áudio,
  o que protege o caminho do microfone na Fase 6.
- (+) O motor de áudio não mudou.
- (−) Primeiro uso de cada tom espera alguns segundos; cada tom ocupa algumas dezenas de MB.
- Requer ffmpeg com o filtro `rubberband` (`ffmpeg -filters | findstr rubberband`).
- A Fase 6 (autotune) deve ler o tom atual (original + semitons) para definir a escala-alvo.
