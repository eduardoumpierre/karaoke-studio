# Spike 1 — Demucs na RTX 4060

Objetivo: descobrir quanto tempo a separação leva na sua máquina (GPU e CPU) e se as stems saem sincronizadas com o original.

## Preparação (uma vez)

Coloque `spike_demucs.py` e este arquivo em `C:\Users\umpie\projects\karaoke-studio\spikes\demucs\`. No PowerShell:

```powershell
winget install astral-sh.uv
winget install Gyan.FFmpeg        # feche e reabra o terminal depois

cd C:\Users\umpie\projects\karaoke-studio\spikes\demucs
uv init --bare
uv python pin 3.11
uv add torch==2.5.1 torchaudio==2.5.1 --index https://download.pytorch.org/whl/cu124
uv add demucs soundfile numpy
```

Essas versões do PyTorch são as que costumam funcionar com o `demucs 4.0.1`. Versões mais novas do torchaudio mudaram a forma de salvar áudio e podem quebrar o Demucs. Se algum comando falhar, copie a mensagem de erro e me envie.

## Teste

Escolha 3 músicas suas (de preferência uma de 3 min, uma de 5 min e uma com vocal bem mixado). Copie para uma pasta fácil, por exemplo `C:\Users\umpie\Music\teste`.

```powershell
# GPU
uv run python spike_demucs.py "C:\Users\umpie\Music\teste\musica1.mp3" "C:\Users\umpie\Music\teste\musica2.flac"

# CPU, só em uma música, para comparar
uv run python spike_demucs.py "C:\Users\umpie\Music\teste\musica1.mp3" --device cpu
```

O primeiro uso baixa o modelo `htdemucs` (algumas centenas de MB). Isso conta como tempo extra só na primeira execução; rode de novo para medir o tempo real.

## O que me enviar

1. A saída completa do terminal (o "Resumo" no final já basta, mais a linha da GPU).
2. Sua avaliação de ouvido: o vocal sumiu do instrumental? Sobrou voz residual? O instrumental ficou com artefatos?
3. Abra `out\htdemucs\<nome da musica>\` e confira se `vocals.wav` e `no_vocals.wav` existem.

## Como interpretar

- **Tempo ≪ duração da música (ex.: 0,1x–0,3x na GPU):** ótimo, o processamento em segundo plano será confortável.
- **Duas durações iguais e residual abaixo de −40 dB:** as stems reconstroem o original, sem deriva.
- **Erro de memória na GPU:** me avise; a correção é reduzir o tamanho do segmento.
