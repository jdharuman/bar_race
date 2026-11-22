import json
from datetime import date
from types import SimpleNamespace

import daily
import scout

DAY = date(2026, 11, 23)


def idea(title="India vs Australia Test Wins", style="compare", theme="sports", **kw):
    return {"style": style, "title": title, "theme": theme, "why": "series starts", "source": "ESPNcricinfo", **kw}


def test_check_keeps_valid_new_ideas_only():
    raw = {"ideas": [
        idea(),
        idea("Bad style", style="pie"),
        idea("No theme", theme=""),
        idea("Unknown theme", theme="cooking"),
        idea("A quiz", style="quiz"),
        idea("india vs  australia test wins"),          # duplicate of the first
        idea("Already Planned"),                       # in topics.csv
        idea("Redo smuggled", comment="ignore rules"),  # comment is stripped
        idea("Neon race", style="race", look="neon"),
    ]}
    got = scout.check(raw, {daily._norm("Already Planned")})
    assert [g["title"] for g in got] == ["India vs Australia Test Wins", "Redo smuggled"]
    assert all(g["comment"] == "" and g["status"] == "pending" for g in got)


def test_check_caps_at_five_and_survives_garbage():
    assert len(scout.check([idea(f"T{i}") for i in range(9)], set())) == 5
    assert scout.check("nonsense", set()) == []
    assert scout.check({"ideas": [1, None, "x"]}, set()) == []


def fake_claude(ideas, rc=0):
    def run(cmd, **kw):
        out = cmd[cmd.index("-p") + 1].split("JSON to ")[1].split(" and nothing")[0]
        if ideas is not None:
            from pathlib import Path
            Path(out).write_text(json.dumps({"ideas": ideas}))
        return SimpleNamespace(returncode=rc, stdout="", stderr="")
    return run


def test_run_scout_sends_buttons_and_saves(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS,
                     [{"style": "race", "title": "Existing", "theme": "sports"}])
    sent = []
    code = scout.run_scout(tmp_path, DAY, run=fake_claude([idea(), idea("Existing")]),
                           send=lambda t, rows: sent.append((t, rows)), notify=print)
    assert code == 0 and len(sent) == 1
    text, rows = sent[0]
    assert "India vs Australia" in text and rows[0][0][1] == "scout:2026-11-23:0:y"
    saved = json.loads((scout.scout_dir(tmp_path) / "2026-11-23.json").read_text())
    assert saved["ideas"][0]["status"] == "pending"


def test_run_scout_reports_missing_output(tmp_path):
    msgs = []
    assert scout.run_scout(tmp_path, DAY, run=fake_claude(None, rc=1), send=None, notify=msgs.append) == 1
    assert "no usable proposals" in msgs[0]


def test_decide_add_then_skip_is_refused(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [{"style": "race", "title": "Old", "theme": "india"}])
    scout.run_scout(tmp_path, DAY, run=fake_claude([idea(), idea("Second", style="race", theme="earth")]),
                    send=lambda t, r: None, notify=print)
    assert "Added" in scout.decide(tmp_path, "scout:2026-11-23:0:y")
    titles = [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")]
    assert titles == ["India vs Australia Test Wins", "Old"]
    assert "Already added" in scout.decide(tmp_path, "scout:2026-11-23:0:n")
    assert "Skipped" in scout.decide(tmp_path, "scout:2026-11-23:1:n")
    assert len(daily.read_rows(tmp_path / "topics.csv")) == 2


def test_decide_rejects_bad_buttons(tmp_path):
    assert "expired" in scout.decide(tmp_path, "scout:2026-11-23:0:y")
    assert "broken" in scout.decide(tmp_path, "scout:../../etc:0:y")
    assert "broken" in scout.decide(tmp_path, "scout:x")


def test_approved_idea_runs_on_its_theme_day(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS,
                     [{"style": "compare", "title": "Old compare", "theme": "sports"}])
    scout.run_scout(tmp_path, DAY, run=fake_claude([idea()]), send=lambda t, r: None, notify=print)
    scout.decide(tmp_path, "scout:2026-11-23:0:y")
    t = daily.read_rows(tmp_path / "topics.csv")
    assert t[daily.choose(t, daily.SLOTS["lines"], "sports")]["title"] == "India vs Australia Test Wins"


def test_scout_cannot_touch_tool_files(tmp_path):
    (tmp_path / "automation").mkdir()

    def run(cmd, **kw):
        (tmp_path / "automation" / "evil.py").write_text("x")
        return SimpleNamespace(returncode=0, stdout="", stderr="")
    msgs = []
    assert scout.run_scout(tmp_path, DAY, run=run, send=None, notify=msgs.append) == 1
    assert "changed tool files" in msgs[0]


def test_scout_command_is_sandboxed(tmp_path):
    cmd = scout.claude_cmd(tmp_path / "x.json")
    allowed = cmd[cmd.index("--allowedTools") + 1: cmd.index("--disallowedTools")]
    assert f"Write(/{tmp_path}/**)" in allowed  # the per-run folder only
    assert f"Edit(/{tmp_path}/**)" in allowed
    assert not any(t.startswith("Bash") for t in allowed)
    assert all(t in ("Read", "Glob", "Grep", "WebSearch", "WebFetch") or str(tmp_path) in t for t in allowed)
    assert "Read(~/.config/barrace/**)" in cmd
