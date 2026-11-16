"""Spike Demucs (Fase 0) - mede tempo e consistencia da separacao vocal/instrumental.

Uso (dentro de spikes/demucs, com o ambiente do SPIKE-DEMUCS.md ativo):
    uv run python spike_demucs.py caminho\\musica1.mp3 caminho\\musica2.flac
    uv run python spike_demucs.py musica.mp3 --device cpu     # para comparar com a CPU

Para cada musica:
  - roda o htdemucs com --two-stems=vocals (vocals + no_vocals)
  - mede o tempo de processamento
  - confere duracao e taxa de amostragem
  - confere se vocals + no_vocals reconstroi o original (residuo em dB)
"""

import argparse
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

OUT_DIR = Path("out")


def run_demucs(track: Path, device: str) -> float:
    cmd = [
        sys.executable, "-m", "demucs",
        "-n", "htdemucs",
        "--two-stems=vocals",
        "--float32",
        "-d", device,
        "-o", str(OUT_DIR),
        str(track),
    ]
    start = time.perf_counter()
    subprocess.run(cmd, check=True)
    return time.perf_counter() - start


def analyse(track: Path) -> None:
    stem_dir = OUT_DIR / "htdemucs" / track.stem
    vocals, sr_v = sf.read(stem_dir / "vocals.wav", dtype="float32", always_2d=True)
    inst, sr_i = sf.read(stem_dir / "no_vocals.wav", dtype="float32", always_2d=True)
    orig, sr_o = sf.read(track, dtype="float32", always_2d=True)

    print(f"  taxa original: {sr_o} Hz | stems: {sr_v} Hz")
    print(f"  duracao original: {len(orig) / sr_o:.2f}s | vocals: {len(vocals) / sr_v:.2f}s | "
          f"instrumental: {len(inst) / sr_i:.2f}s")
    print(f"  amostras vocals vs instrumental: {len(vocals)} vs {len(inst)} "
          f"({'iguais' if len(vocals) == len(inst) else 'DIFERENTES'})")

    if sr_o != sr_v:
        print("  (taxa diferente do original: pulando teste de reconstrucao)")
        return

    n = min(len(orig), len(vocals), len(inst))
    channels = min(orig.shape[1], vocals.shape[1], inst.shape[1])
    recon = vocals[:n, :channels] + inst[:n, :channels]
    ref = orig[:n, :channels]
    resid = ref - recon
    rms_ref = float(np.sqrt(np.mean(ref ** 2))) or 1e-12
    rms_res = float(np.sqrt(np.mean(resid ** 2))) or 1e-12
    print(f"  residuo da reconstrucao: {20 * np.log10(rms_res / rms_ref):.1f} dB "
          f"(quanto mais negativo, melhor)")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("tracks", nargs="+", type=Path)
    parser.add_argument("--device", default="cuda", choices=["cuda", "cpu"])
    args = parser.parse_args()

    try:
        import torch
        print(f"PyTorch {torch.__version__} | CUDA disponivel: {torch.cuda.is_available()}")
        if torch.cuda.is_available():
            print(f"GPU: {torch.cuda.get_device_name(0)}")
        if args.device == "cuda" and not torch.cuda.is_available():
            print("CUDA indisponivel: usando a CPU.")
            args.device = "cpu"
    except ImportError:
        print("PyTorch nao instalado.")
        sys.exit(1)

    results = []
    for track in args.tracks:
        info = sf.info(track)
        duration = info.frames / info.samplerate
        print(f"\n== {track.name} ({duration:.0f}s, {info.samplerate} Hz) | dispositivo: {args.device}")
        elapsed = run_demucs(track, args.device)
        print(f"  tempo de processamento: {elapsed:.1f}s ({elapsed / duration:.2f}x a duracao)")
        analyse(track)
        results.append((track.name, duration, elapsed))

    print("\n== Resumo")
    for name, duration, elapsed in results:
        print(f"  {name}: {duration:.0f}s de musica em {elapsed:.1f}s")


if __name__ == "__main__":
    main()
