from __future__ import annotations

import logging

from fastapi.testclient import TestClient

from app.config import TOKEN_HEADER, Settings
from app.main import create_app


def test_logs_requests_but_not_polling(tmp_path, caplog) -> None:
    settings = Settings(data_dir=tmp_path, token="tok")
    with TestClient(create_app(settings)) as client, caplog.at_level(logging.INFO, "karaoke.http"):
        headers = {TOKEN_HEADER: "tok"}
        assert client.get("/songs", headers=headers).status_code == 200
        assert client.get("/jobs", headers=headers).status_code == 200
    lines = [r.getMessage() for r in caplog.records if r.name == "karaoke.http"]
    assert any(line.startswith("-> GET /songs") for line in lines)
    assert any(line.startswith("<- GET /songs 200") for line in lines)
    assert not any("/jobs" in line for line in lines)
