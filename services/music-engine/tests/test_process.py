"""Teste ponta a ponta do protocolo com o Tauri: sobe `python -m app` de verdade."""

import json
import os
import subprocess
import sys
import urllib.request

from app.config import TOKEN_HEADER


def test_sidecar_announces_port_serves_health_and_exits_on_stdin_close(tmp_path):
    env = {**os.environ, "KARAOKE_TOKEN": "p" * 24, "KARAOKE_DATA_DIR": str(tmp_path)}
    proc = subprocess.Popen(
        [sys.executable, "-m", "app"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True,
    )
    try:
        line = proc.stdout.readline()
        info = json.loads(line)
        assert info["event"] == "ready"
        req = urllib.request.Request(
            f"http://127.0.0.1:{info['port']}/health", headers={TOKEN_HEADER: "p" * 24}
        )
        with urllib.request.urlopen(req, timeout=5) as resp:
            assert json.load(resp)["status"] == "ok"
        proc.stdin.close()
        assert proc.wait(timeout=10) == 0
    finally:
        if proc.poll() is None:
            proc.kill()
    assert (tmp_path / "logs" / "music-engine.log").exists()


def test_sidecar_respects_fixed_port(tmp_path):
    import socket

    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    env = {
        **os.environ,
        "KARAOKE_TOKEN": "p" * 24,
        "KARAOKE_DATA_DIR": str(tmp_path),
        "KARAOKE_PORT": str(port),
    }
    proc = subprocess.Popen(
        [sys.executable, "-m", "app"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
        text=True,
    )
    try:
        assert json.loads(proc.stdout.readline()) == {"event": "ready", "port": port}
        proc.stdin.close()
        assert proc.wait(timeout=10) == 0
    finally:
        if proc.poll() is None:
            proc.kill()
