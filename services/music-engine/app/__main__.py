"""Ponto de entrada: `python -m app`.

Protocolo com o Tauri:
  - lê KARAOKE_TOKEN (e opcionalmente KARAOKE_DATA_DIR) do ambiente;
  - escolhe uma porta livre em 127.0.0.1 (ou usa KARAOKE_PORT, em desenvolvimento);
  - imprime uma linha JSON no stdout quando está pronto: {"event":"ready","port":N};
  - encerra quando o stdin é fechado (o Tauri morreu ou encerrou o app).
"""

from __future__ import annotations

import json
import logging
import os
import socket
import sys
import threading

import uvicorn

from .config import Settings
from .logging_setup import setup_logging
from .main import create_app

log = logging.getLogger("music-engine")


class AnnouncingServer(uvicorn.Server):
    """Avisa o Tauri (stdout) só depois que o servidor realmente aceita conexões."""

    def __init__(self, config: uvicorn.Config, port: int) -> None:
        super().__init__(config)
        self._port = port

    async def startup(self, sockets=None) -> None:  # type: ignore[override]
        await super().startup(sockets=sockets)
        if not self.should_exit:
            print(json.dumps({"event": "ready", "port": self._port}), flush=True)


def _watch_stdin(server: uvicorn.Server) -> None:
    try:
        while sys.stdin.read(1024):
            pass
    except (OSError, ValueError):
        pass
    log.info("stdin fechado: encerrando.")
    server.should_exit = True


def main() -> int:
    settings = Settings.from_env()
    setup_logging(settings.log_dir)
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    # KARAOKE_PORT fixa a porta (útil para rodar a UI no navegador); o padrão é uma porta livre.
    sock.bind(("127.0.0.1", int(os.environ.get("KARAOKE_PORT", "0"))))
    port = sock.getsockname()[1]
    config = uvicorn.Config(create_app(settings), log_config=None, access_log=False)
    server = AnnouncingServer(config, port)
    threading.Thread(target=_watch_stdin, args=(server,), daemon=True).start()
    log.info("music-engine iniciando em 127.0.0.1:%d", port)
    server.run(sockets=[sock])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
