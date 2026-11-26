import json
from datetime import date, datetime

import daily
import growth

import test_bot


def topics(*titles, style="race"):
    return [{c: "" for c in daily.TOPIC_COLS} | {"style": style, "title": t, "theme": "economy"}
            for t in titles]


def setup(tmp_path, rows):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, rows)
    daily.write_rows(tmp_path / "topics_done.csv", daily.DONE_COLS, [])


def fake_stats(root, days):
    return {"channel": {"title": "GlobeMetrics", "subscriberCount": "21"},
            "range": ["2026-10-29", "2026-11-26"], "videos": {}, "per_video": [], "per_day": [],
            "traffic": [], "audience": {}, "kinds": {}, "analytics_error": ""}


PLAN = {"report": "Race charts on money work best.",
        "tips": ["Pin a comment asking for the next country", ""],
        "promote": [{"title": "richest people in asia", "why": "money races win"},
                    {"title": "Not In The List"}],
        "add": [{"style": "race", "title": "Most Valuable Football Clubs", "theme": "sports",
                 "why": "CL knockouts", "source": "Forbes"},
                {"style": "quiz", "title": "Guess the Flag: Europe #9", "theme": "sports"},
                {"style": "race", "title": "Old Topic", "theme": "economy"}],
        "drop": [{"title": "Largest Rivers", "why": "geo pillars weak"}]}


def run(tmp_path, plan=PLAN, question=""):
    seen, sent = {}, []

    def claude(cmd, **kw):
        seen["cmd"] = cmd
        (growth.growth_dir(tmp_path) / "2026-11-29T19-00-00" / "plan.json").write_text(json.dumps(plan))
        return type("R", (), {"stdout": "", "stderr": "", "returncode": 0})()
    code = growth.run_growth(tmp_path, date(2026, 11, 29), question, run=claude, collect=fake_stats,
                             send=lambda text, rows: sent.append((text, rows)),
                             notify=sent.append, now=lambda: datetime(2026, 11, 29, 19))
    return code, seen, sent


def test_plan_is_checked_applied_and_reported_with_undo(tmp_path):
    setup(tmp_path, [*topics("A", "Old Topic", "Richest People in Asia"),
                     *topics("Largest Rivers", style="pillars")])
    code, seen, sent = run(tmp_path, question="why no subscribers?")
    assert code == 0
    assert "Bash" in seen["cmd"][seen["cmd"].index("--disallowedTools") + 1:]
    d = growth.growth_dir(tmp_path) / "2026-11-29T19-00-00"
    assert (d / "question.txt").read_text() == "why no subscribers?"
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == [
        "Most Valuable Football Clubs", "Richest People in Asia", "A", "Old Topic"]
    text, rows = sent[-1]
    assert "Race charts on money" in text and "⬆️ Richest People in Asia – money races win" in text
    assert "➕ Most Valuable Football Clubs (race, sports)" in text and "➖ Largest Rivers" in text
    assert "Pin a comment" in text and "Guess the Flag" not in text
    assert rows == [[("↩️ Undo topic changes", "growth:2026-11-29T19-00-00:undo")]]
    assert "Largest Rivers" in (growth.growth_dir(tmp_path) / "dropped.csv").read_text()

    assert growth.undo(tmp_path, "growth:2026-11-29T19-00-00:undo").startswith("↩️ Undone")
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == [
        "Richest People in Asia", "A", "Old Topic", "Largest Rivers"]
    assert growth.undo(tmp_path, "growth:2026-11-29T19-00-00:undo") == "Already undone."
    assert growth.undo(tmp_path, "growth:2026-11-29T19-00-00/../x:undo") == "That button is broken."


def test_no_plan_or_no_stats_changes_nothing(tmp_path):
    setup(tmp_path, topics("A"))
    code, _, sent = run(tmp_path, plan="not json at all")
    assert code == 0 and sent[-1][1] == []  # a non-dict plan: report only, no changes
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == ["A"]

    def no_login(root, days):
        raise SystemExit("No stats login")
    msgs = []
    assert growth.run_growth(tmp_path, date(2026, 11, 29), collect=no_login, notify=msgs.append,
                             now=lambda: datetime(2026, 11, 29, 20)) == 1
    assert "could not read channel stats" in msgs[0]


def test_bot_grow_starts_a_background_review_and_undo_button_works(tmp_path):
    setup(tmp_path, topics("A"))
    calls = []
    b, tg, _ = test_bot.make(tmp_path)
    b.start = lambda root, args, log_dir, cmd="request": calls.append((cmd, args)) or test_bot.FakeChild()
    b.handle(test_bot.msg("/grow why are quizzes better?"))
    assert calls == [("growth", ["--question=why are quizzes better?"])]
    assert "Growth review started" in tg.texts()[-1]
    b.handle(test_bot.tap("growth:2026-11-29T19-00-00:undo"))
    assert tg.texts()[-1] == "Nothing to undo for that review."


def test_topic_changes_are_committed_to_the_state_repo(tmp_path, monkeypatch):
    import subprocess
    state = tmp_path / "state"
    monkeypatch.setenv("BARRACE_STATE", str(state))
    root = tmp_path / "repo"
    root.mkdir()
    setup(root, topics("A", "Richest People in Asia"))
    assert daily.commit_state(root, "first") is True
    assert daily.commit_state(root, "nothing changed") is False
    run(root)  # the growth review applies its plan and commits it
    log = subprocess.run(["git", "-C", str(state), "log", "--format=%s"], capture_output=True,
                         text=True).stdout.splitlines()
    assert log == ["growth review 2026-11-29T19-00-00: +2 added, 1 promoted, -0 dropped", "first"]
    assert "Most Valuable Football Clubs" in (state / "topics.csv").read_text()
    assert (state / "dropped.csv").exists() is False  # nothing dropped: no file yet
