import pytest


@pytest.fixture(autouse=True)
def classic_mode(monkeypatch):
    """Most tests were written for main + Short pairs and no quiz fill; tests of the
    Shorts-only default and the quiz fill change these env vars themselves."""
    monkeypatch.setenv("BARRACE_MAIN", "on")
    monkeypatch.setenv("BARRACE_QUIZ", "off")
    monkeypatch.setenv("BARRACE_QUOTA", "10000")
    monkeypatch.delenv("TELEGRAM_BOT_TOKEN", raising=False)  # never message the real chat
    monkeypatch.delenv("TELEGRAM_CHAT_ID", raising=False)
