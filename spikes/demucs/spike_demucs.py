"""Spike Demucs (Fase 0) - mede tempo e consistencia da separacao vocal/instrumental.

Uso (dentro de spikes/demucs):
    uv run python spike_demucs.py caminho\\musica1.mp3 caminho\\musica2.flac
    uv run python spike_demucs.py musica.mp3 --device cpu     # para comparar com a CPU

Para cada musica:
  - roda o htdemucs com --two-stems=vocals (vocals + no_vocals)
  - mede o tempo de processamento (modelo ja baixado: rode duas vezes)
  - confere duracao e taxa de amostragem
  - converte o original para a taxa das stems (via ffmpeg) e confere se
    vocals + no_vocals reconstroi o original, medindo tambem o atraso (lag) entre eles
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


def resample_with_ffmpeg(track: Path, sr: int) -> np.ndarray:
    tmp = OUT_DIR / f"_{track.stem}_{sr}.wav"
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-i", str(track),
         "-ar", str(sr), "-c:a", "pcm_f32le", str(tmp)],
        check=True,
    )
    data, _ = sf.read(tmp, dtype="float32", always_2d=True)
    tmp.unlink(missing_ok=True)
    return data


def best_lag(ref: np.ndarray, rec: np.ndarray, sr: int, frac: float = 0.25,
             max_lag: int = 4000) -> int:
    """Atraso (em amostras) que melhor alinha rec com ref, medido numa janela de 20 s
    que comeca em `frac` da musica. Positivo: rec esta adiantado; negativo: atrasado."""
    n = min(len(ref), len(rec))
    start = int(n * frac)
    length = min(sr * 20, n - start)
    a = ref[start:start + length].mean(axis=1)
    b = rec[start:start + length].mean(axis=1)
    size = 1 << (len(a) + len(b)).bit_length()
    corr = np.fft.irfft(np.fft.rfft(a, size) * np.conj(np.fft.rfft(b, size)), size)
    lags = np.concatenate([np.arange(0, max_lag + 1), np.arange(-max_lag, 0)])
    vals = np.concatenate([corr[:max_lag + 1], corr[-max_lag:]])
    return int(lags[int(np.argmax(vals))])


def residual_db(ref: np.ndarray, rec: np.ndarray) -> float:
    rms_ref = float(np.sqrt(np.mean(ref ** 2))) or 1e-12
    rms_res = float(np.sqrt(np.mean((ref - rec) ** 2))) or 1e-12
    return 20 * np.log10(rms_res / rms_ref)


def analyse(track: Path) -> None:
    stem_dir = OUT_DIR / "htdemucs" / track.stem
    vocals, sr_v = sf.read(stem_dir / "vocals.wav", dtype="float32", always_2d=True)
    inst, sr_i = sf.read(stem_dir / "no_vocals.wav", dtype="float32", always_2d=True)
    info = sf.info(track)
    sr_o = info.samplerate

    print(f"  taxa original: {sr_o} Hz | stems: {sr_v} Hz")
    print(f"  duracao original: {info.frames / sr_o:.3f}s | vocals: {len(vocals) / sr_v:.3f}s | "
          f"instrumental: {len(inst) / sr_i:.3f}s")
    print(f"  amostras vocals vs instrumental: {len(vocals)} vs {len(inst)} "
          f"({'iguais' if len(vocals) == len(inst) else 'DIFERENTES'})")

    if sr_o == sr_v:
        orig, _ = sf.read(track, dtype="float32", always_2d=True)
    else:
        print(f"  convertendo o original para {sr_v} Hz com ffmpeg para comparar...")
        orig = resample_with_ffmpeg(track, sr_v)

    n = min(len(orig), len(vocals), len(inst))
    channels = min(orig.shape[1], vocals.shape[1], inst.shape[1])
    ref = orig[:n, :channels]
    rec = vocals[:n, :channels] + inst[:n, :channels]

    print(f"  residuo sem alinhamento: {residual_db(ref, rec):.1f} dB")

    fracs = (0.05, 0.5, 0.9)
    lags = [best_lag(ref, rec, sr_v, f) for f in fracs]
    print("  atraso por posicao: " + " | ".join(
        f"{int(f * 100)}%: {lg} amostras ({1000 * lg / sr_v:.2f} ms)" for f, lg in zip(fracs, lags)))
    print(f"  variacao entre inicio e fim: {max(lags) - min(lags)} amostras "
          f"({'constante' if max(lags) - min(lags) <= 2 else 'HA DERIVA'})")
    lag = lags[1]
    if lag > 0:
        ref_a, rec_a = ref[lag:], rec[:len(rec) - lag]
    elif lag < 0:
        ref_a, rec_a = ref[:len(ref) + lag], rec[-lag:]
    else:
        ref_a, rec_a = ref, rec
    print(f"  atraso detectado entre original e stems: {lag} amostras ({1000 * lag / sr_v:.2f} ms)")
    print(f"  residuo apos alinhar: {residual_db(ref_a, rec_a):.1f} dB "
          f"(quanto mais negativo, melhor; perto de -25 dB e comum em separadores neurais)")


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
