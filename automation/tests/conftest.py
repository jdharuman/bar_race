import pytest


@pytest.fixture(autouse=True)
def classic_mode(monkeypatch, tmp_path_factory):
    """Most tests were written for main + Short pairs and no quiz fill; tests of the
    Shorts-only default and the quiz fill change these env vars themselves."""
    monkeypatch.setenv("BARRACE_MAIN", "on")
    monkeypatch.setenv("BARRACE_QUIZ", "off")
    monkeypatch.setenv("BARRACE_QUOTA", "10000")
    monkeypatch.setenv("BARRACE_SLOTS", "race,pillars,lines")
    monkeypatch.setenv("BARRACE_STATE", str(tmp_path_factory.mktemp("state")))  # never ~/barrace-state
    monkeypatch.delenv("TELEGRAM_BOT_TOKEN", raising=False)  # never message the real chat
    monkeypatch.delenv("TELEGRAM_CHAT_ID", raising=False)
