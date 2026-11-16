# 0003. Separação vocal/instrumental

Status: aceita (spike 1)

**Decisão:** Demucs `htdemucs --two-stems=vocals` em PyTorch 2.5.1+cu124 (Python 3.11) na RTX 4060. Na importação, normalizar a música para WAV estéreo de 44,1 kHz com FFmpeg antes de separar (o início do MP3 tem deslocamento de ~23 ms que a separação transforma em silêncio). Stems guardadas em FLAC. A soma das stems tem resíduo de cerca de −26 dB, o que é normal.

**Mac (M1 Max):** a testar depois, com `PYTORCH_ENABLE_MPS_FALLBACK=1`.
