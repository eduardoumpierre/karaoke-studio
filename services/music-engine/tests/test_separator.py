"""DemucsSeparator com um programa falso no lugar do Demucs (sem PyTorch nos testes)."""

import sys
import threading
import time

import pytest

from app.services.audio_tools import AudioToolError, Cancelled
from app.services.separator import DemucsSeparator

STUB = r"""
import sys, time, pathlib
args = sys.argv[1:]
out = pathlib.Path(args[args.index("-o") + 1])
mode = args[-2]
if mode == "fail":
    sys.stderr.write("RuntimeError: CUDA out of memory\n"); sys.exit(1)
sys.stderr.write("Selected model is a bag of 1 models.\n")
for pct in (0, 25, 50, 75, 100):
    sys.stderr.write(f"\r {pct:3d}%|####| {pct}/100 [00:01<00:00]"); sys.stderr.flush()
    if mode == "slow": time.sleep(5)
if mode != "nofiles":
    d = out / "htdemucs" / "input"; d.mkdir(parents=True)
    (d / "vocals.wav").write_bytes(b"v"); (d / "no_vocals.wav").write_bytes(b"i")
"""


class StubDemucs(DemucsSeparator):
    def __init__(self, tmp_path, mode):
        super().__init__(tmp_path)
        self.mode = mode
        self.script = tmp_path / "stub.py"
        self.script.write_text(STUB)

    def command(self, wav, out_dir):
        return [sys.executable, str(self.script), "-o", str(out_dir), self.mode, str(wav)]


def test_reports_progress_and_finds_stems(tmp_path):
    seen: list[float] = []
    stems = StubDemucs(tmp_path, "ok").separate(
        tmp_path / "input.wav", tmp_path / "out", seen.append, threading.Event()
    )
    assert seen == [0.0, 0.25, 0.5, 0.75, 1.0]
    assert stems.vocals.read_bytes() == b"v" and stems.instrumental.read_bytes() == b"i"


def test_failure_includes_stderr_tail(tmp_path):
    with pytest.raises(AudioToolError, match="CUDA out of memory"):
        StubDemucs(tmp_path, "fail").separate(
            tmp_path / "input.wav", tmp_path / "out", lambda _: None, threading.Event()
        )


def test_missing_outputs_are_an_error(tmp_path):
    with pytest.raises(AudioToolError, match="não gerou"):
        StubDemucs(tmp_path, "nofiles").separate(
            tmp_path / "input.wav", tmp_path / "out", lambda _: None, threading.Event()
        )


def test_cancel_kills_the_process_quickly(tmp_path):
    cancel = threading.Event()
    threading.Timer(0.6, cancel.set).start()
    start = time.time()
    with pytest.raises(Cancelled):
        StubDemucs(tmp_path, "slow").separate(
            tmp_path / "input.wav", tmp_path / "out", lambda _: None, cancel
        )
    assert time.time() - start < 4


def test_real_command_shape(tmp_path, monkeypatch):
    monkeypatch.delenv("KARAOKE_DEVICE", raising=False)
    cmd = DemucsSeparator(tmp_path, device="cuda").command(tmp_path / "a.wav", tmp_path / "o")
    assert cmd[1:3] == ["-m", "demucs"]
    assert "--two-stems=vocals" in cmd and "htdemucs" in cmd
    assert cmd[cmd.index("-d") + 1] == "cuda"
    monkeypatch.setattr(sys, "platform", "linux")
    assert "-d" not in DemucsSeparator(tmp_path, device=None).command(tmp_path / "a.wav", tmp_path)


def test_macos_defaults_to_cpu(tmp_path, monkeypatch):
    # O htdemucs falha na GPU (MPS) do Mac com o torch fixado; ver default_device().
    monkeypatch.delenv("KARAOKE_DEVICE", raising=False)
    monkeypatch.setattr(sys, "platform", "darwin")
    cmd = DemucsSeparator(tmp_path).command(tmp_path / "a.wav", tmp_path / "o")
    assert cmd[cmd.index("-d") + 1] == "cpu"
    monkeypatch.setenv("KARAOKE_DEVICE", "mps")
    cmd = DemucsSeparator(tmp_path).command(tmp_path / "a.wav", tmp_path / "o")
    assert cmd[cmd.index("-d") + 1] == "mps"


def test_unavailable_reason_mentions_install(tmp_path, monkeypatch):
    import importlib.util

    monkeypatch.setattr(importlib.util, "find_spec", lambda name: None)
    assert "uv sync --group ml" in DemucsSeparator(tmp_path).unavailable_reason()
