import pytest
from fastapi.testclient import TestClient

from app.config import Settings
from app.main import create_app

TOKEN = "t" * 32


@pytest.fixture
def client(tmp_path):
    app = create_app(Settings(token=TOKEN, data_dir=tmp_path))
    with TestClient(app) as c:
        yield c
