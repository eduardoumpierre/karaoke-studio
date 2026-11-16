from app.config import TOKEN_HEADER

from .conftest import TOKEN


def test_health_ok_with_token(client):
    r = client.get("/health", headers={TOKEN_HEADER: TOKEN})
    assert r.status_code == 200
    body = r.json()
    assert body["status"] == "ok"
    assert body["version"]
    assert body["uptime_seconds"] >= 0


def test_health_rejects_missing_token(client):
    assert client.get("/health").status_code == 401


def test_health_rejects_wrong_token(client):
    assert client.get("/health", headers={TOKEN_HEADER: "errado" * 6}).status_code == 401


def test_cors_allows_tauri_origin(client):
    r = client.options(
        "/health",
        headers={
            "Origin": "http://tauri.localhost",
            "Access-Control-Request-Method": "GET",
            "Access-Control-Request-Headers": TOKEN_HEADER,
        },
    )
    assert r.status_code == 200
    assert r.headers["access-control-allow-origin"] == "http://tauri.localhost"


def test_cors_blocks_other_origins(client):
    r = client.options(
        "/health",
        headers={"Origin": "http://evil.example", "Access-Control-Request-Method": "GET"},
    )
    assert "access-control-allow-origin" not in r.headers
