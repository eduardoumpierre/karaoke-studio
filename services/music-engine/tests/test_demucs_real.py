"""Teste com o Demucs de verdade (GPU ou CPU). Só roda se o grupo `ml` estiver instalado:

    uv sync --group ml
    uv run pytest tests/test_demucs_real.py -s

Confere o contrato da Fase 3: as duas stems têm exatamente o mesmo número de amostras do áudio
normalizado e a soma delas fica próxima do original.

Atenção: o Demucs monta o instrumental somando as outras fontes (bateria, baixo, outros), não
fazendo "mix - voz"; então a soma só reconstrói o original até o erro do modelo. Com o sinal
sintético (senoides, fora do que o modelo conhece) o resíduo fica perto de -15 dB, por isso o
limite é de sanidade (-10 dB). Para medir com música de verdade:

    $env:KARAOKE_TEST_SONG = "C:\\caminho\\musica.mp3"; uv run pytest tests/test_demucs_real.py -s
"""

import importlib.util
import os
import subprocess
import threading

import pytest

from app.services.audio_tools import encode_flac, ffmpeg_command, normalize_to_wav
from app.services.separator import DemucsSeparator

pytestmark = pytest.mark.skipif(
    importlib.util.find_spec("demucs") is None or importlib.util.find_spec("numpy") is None,
    reason="Demucs não instalado (uv sync --group ml)",
)


def decode(path):
    import numpy as np

    raw = subprocess.run(
        [ffmpeg_command(), "-v", "error", "-i", str(path), "-f", "f32le", "-ar", "44100"]
        + ["-ac", "2", "-"],
        capture_output=True,
        check=True,
    ).stdout
    return np.frombuffer(raw, dtype="<f4").reshape(-1, 2)


def test_demucs_stems_match_original(tmp_path):
    import numpy as np

    real = os.environ.get("KARAOKE_TEST_SONG")
    if real:
        src = tmp_path / "mix.wav"  # só os primeiros 30 s, para ser rápido
        subprocess.run(
            [ffmpeg_command(), "-y", "-v", "error", "-i", real, "-t", "30", str(src)], check=True
        )
    else:
        src = tmp_path / "mix.mp3"
        # 12 s de sinal sintético (acorde), num MP3 mono de 48 kHz (força a normalização).
        expr = "sin(2*PI*220*t)+0.6*sin(2*PI*277*t)+0.5*sin(2*PI*330*t)"
        subprocess.run(
            [ffmpeg_command(), "-y", "-v", "error", "-f", "lavfi", "-i"]
            + [f"aevalsrc={expr}*0.3:d=12:s=48000", str(src)],
            check=True,
        )
    cancel = threading.Event()
    wav = tmp_path / "input.wav"
    normalize_to_wav(src, wav, cancel)

    seen: list[float] = []
    sep = DemucsSeparator(tmp_path)
    assert sep.unavailable_reason() is None
    stems = sep.separate(wav, tmp_path / "out", seen.append, cancel)
    assert seen and max(seen) <= 1.0
    print(f"progresso: {len(seen)} atualizações, última {seen[-1]:.0%}")

    flacs = []
    for name, stem in (("vocals", stems.vocals), ("instrumental", stems.instrumental)):
        flac = tmp_path / f"{name}.flac"
        encode_flac(stem, flac, cancel)
        flacs.append(flac)

    ref, voc, ins = decode(wav), decode(flacs[0]), decode(flacs[1])
    assert len(voc) == len(ref) and len(ins) == len(ref), "duração diferente do original"
    residual = np.sqrt(np.mean((ref - (voc + ins)) ** 2)) / np.sqrt(np.mean(ref**2))
    db = 20 * np.log10(residual + 1e-12)
    print(f"resíduo da soma: {db:.1f} dB (quanto menor, melhor)")
    assert db < -10
