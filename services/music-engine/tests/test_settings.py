import pytest

from app.config import Settings


def test_settings_requires_a_long_token(monkeypatch):
    monkeypatch.delenv("KARAOKE_TOKEN", raising=False)
    with pytest.raises(RuntimeError):
        Settings.from_env()
    monkeypatch.setenv("KARAOKE_TOKEN", "curto")
    with pytest.raises(RuntimeError):
        Settings.from_env()


def test_settings_reads_data_dir(monkeypatch, tmp_path):
    monkeypatch.setenv("KARAOKE_TOKEN", "x" * 20)
    monkeypatch.setenv("KARAOKE_DATA_DIR", str(tmp_path))
    s = Settings.from_env()
    assert s.data_dir == tmp_path
    assert s.log_dir == tmp_path / "logs"
