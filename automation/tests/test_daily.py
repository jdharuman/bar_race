import csv
import json
import os
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

import daily

DAILY = str(Path(daily.__file__))
TODAY = date(2026, 11, 22)


def write_topics(root, rows):
    daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, rows)


def row(style="race", title="Largest Economies", url="", comment="", look=""):
    return {"style": style, "title": title, "youtube_url": url, "comment": comment, "look": look}


def cli(root, *args):
    return subprocess.run([sys.executable, DAILY, "--root", str(root), *args],
                          capture_output=True, text=True)


def test_pick_creates_folder_and_topic_json(tmp_path):
    write_topics(tmp_path, [row()])
    out = daily.pick(tmp_path, TODAY)
    d = Path(out["dir"])
    assert d == tmp_path / "videos" / "2026-11-22-largest-economies"
    topic = json.loads((d / "topic.json").read_text())
    assert topic["row"]["title"] == "Largest Economies"
    assert topic["redo_of"] is None
    assert daily.read_rows(tmp_path / "topics.csv") == [row()]


def test_pick_same_day_twice_suffixes(tmp_path):
    (tmp_path / "videos" / "2026-11-22-largest-economies").mkdir(parents=True)
    write_topics(tmp_path, [row()])
    out = daily.pick(tmp_path, TODAY)
    assert out["dir"].endswith("2026-11-22-largest-economies-2")


def test_slug_from_url():
    assert daily.slugify(row(title="", url="https://youtu.be/abc123")) == "yt-abc123"
    assert daily.slugify(row(title="GDP: Top 10 (1960–2025)!")) == "gdp-top-10-1960-2025"


def test_video_id_forms():
    assert daily.video_id("https://www.youtube.com/watch?v=q9EjG56OjN4&t=3") == "q9EjG56OjN4"
    assert daily.video_id("https://youtube.com/shorts/abcDEF") == "abcDEF"
    assert daily.video_id("") is None


def test_pick_empty_exit3(tmp_path):
    write_topics(tmp_path, [])
    r = cli(tmp_path, "pick")
    assert r.returncode == 3
    assert json.loads(r.stdout)["empty"] is True


def test_pick_locked_exit4(tmp_path):
    write_topics(tmp_path, [row()])
    (tmp_path / "videos").mkdir()
    (tmp_path / "videos" / ".lock").write_text("x")
    r = cli(tmp_path, "pick")
    assert r.returncode == 4
    out = json.loads(r.stdout)
    assert out["locked"] is True and out["holder"]["kind"] == "daily"  # old plain lock = daily
    assert not (tmp_path / "videos" / "2026-11-22-largest-economies").exists()


def test_stale_lock_ignored(tmp_path):
    write_topics(tmp_path, [row()])
    lock = tmp_path / "videos" / ".lock"
    lock.parent.mkdir()
    lock.write_text("x")
    old = time.time() - 7 * 3600
    os.utime(lock, (old, old))
    out = daily.pick(tmp_path, TODAY)
    assert Path(out["dir"]).is_dir()


def test_invalid_row_moves_to_done(tmp_path):
    write_topics(tmp_path, [row(style="bars", title="X")])
    r = cli(tmp_path, "pick")
    assert r.returncode == 2
    assert daily.read_rows(tmp_path / "topics.csv") == []
    done = daily.read_rows(tmp_path / "topics_done.csv")
    assert done[0]["status"] == "failed"
    assert "style" in done[0]["error"]
    assert not (tmp_path / "videos" / ".lock").exists()


def test_redo_copies_previous(tmp_path):
    prev = tmp_path / "videos" / "2026-11-20-largest-economies"
    (prev / "out").mkdir(parents=True)
    (prev / "data.csv").write_text("year,name,value\n")
    (prev / "out" / "main.mp4").write_text("v")
    (prev / "upload.json").write_text("{}")
    (prev / "research_notes.md").write_text("## Redo fixes\n- old\n")
    done = {**row(), "finished_at": "x", "folder": str(prev), "status": "uploaded",
            "main_video_id": "m", "short_video_id": "s", "error": ""}
    daily.write_rows(tmp_path / "topics_done.csv", daily.DONE_COLS, [done])
    write_topics(tmp_path, [row(title="largest  economies", comment="India missing")])
    out = daily.pick(tmp_path, TODAY)
    d = Path(out["dir"])
    assert (d / "data.csv").exists()
    assert not (d / "out").exists()
    assert not (d / "upload.json").exists()
    assert out["redo_of"] == str(prev)
    assert not (d / "research_notes.md").exists()
    assert (d / "research_notes.prev.md").exists()


def picked(tmp_path, rows):
    write_topics(tmp_path, rows)
    daily.take_lock(tmp_path)
    return Path(daily.pick(tmp_path, TODAY)["dir"])


def test_finish_moves_exact_row(tmp_path):
    a, b, c = row(title="A"), row(title="B"), row(title="C")
    d = picked(tmp_path, [a, b])
    write_topics(tmp_path, [c, a, b])  # user prepended a row mid-run
    (d / "upload.json").write_text(json.dumps({"main": "m1", "short": "s1"}))
    daily.finish(tmp_path, d, "uploaded")
    assert daily.read_rows(tmp_path / "topics.csv") == [c, b]
    done = daily.read_rows(tmp_path / "topics_done.csv")
    assert [r["title"] for r in done] == ["A"]
    assert done[0]["status"] == "uploaded"
    assert done[0]["folder"] == str(d)
    assert (done[0]["main_video_id"], done[0]["short_video_id"]) == ("m1", "s1")


def test_finish_failed_records_error_and_partial_ids(tmp_path):
    d = picked(tmp_path, [row()])
    (d / "error.txt").write_text("research: boom\n")
    (d / "upload.json").write_text(json.dumps({"main": "m1"}))
    out = daily.finish(tmp_path, d, "failed")
    done = daily.read_rows(tmp_path / "topics_done.csv")[0]
    assert done["status"] == "failed"
    assert "boom" in done["error"]
    assert done["main_video_id"] == "m1"
    assert "FAILED" in out["message"] and "boom" in out["message"]


def test_finish_message_links(tmp_path):
    d = picked(tmp_path, [row()])
    (d / "upload.json").write_text(json.dumps({"main": "m1", "short": "s1"}))
    out = daily.finish(tmp_path, d, "uploaded")
    assert "https://studio.youtube.com/video/m1/edit" in out["message"]
    assert "https://studio.youtube.com/video/s1/edit" in out["message"]


def test_finish_releases_lock(tmp_path):
    d = picked(tmp_path, [row()])
    r = cli(tmp_path, "finish", str(d), "--status", "uploaded")
    assert r.returncode == 0, r.stderr
    assert not (tmp_path / "videos" / ".lock").exists()
    assert json.loads(r.stdout)["status"] == "uploaded"


class FakeRun:
    """Stands in for subprocess.run; returns queued (returncode, stdout) per call."""
    def __init__(self, results, on_call=None):
        self.results, self.calls, self.on_call = list(results), [], on_call

    def __call__(self, cmd, **kw):
        self.calls.append(cmd)
        self.cwds = getattr(self, "cwds", []) + [kw.get("cwd")]
        self.envs = getattr(self, "envs", []) + [kw.get("env")]
        if self.on_call:
            self.on_call(cmd)
        code, out = self.results.pop(0)
        return subprocess.CompletedProcess(cmd, code, out, "")


def test_claude_cmd_uses_sonnet_and_skill(tmp_path):
    cmd = daily.claude_cmd(tmp_path / "videos" / "x", tmp_path)
    assert cmd[:2] == ["claude", "-p"]
    assert cmd[cmd.index("--model") + 1] == "sonnet"
    assert "barrace-research/SKILL.md" in cmd[2]
    assert str(tmp_path / "videos" / "x") in cmd[2]


def test_research_retries_on_usage_limit(tmp_path):
    d = tmp_path / "v"
    d.mkdir()

    def make_files(cmd):
        if cmd[0] == "claude":
            (d / "metadata.json").write_text("{}")
            (d / "data.csv").write_text("x")
    run = FakeRun([(1, "Claude usage limit reached"), (0, "{}"), (0, "ok")], make_files)
    slept = []
    daily.research(tmp_path, d, run=run, sleep=slept.append)
    assert slept == [1800]
    assert [c[0] for c in run.calls].count("claude") == 2


def test_research_fails_without_metadata(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    run = FakeRun([(0, "{}")])
    try:
        daily.research(tmp_path, d, run=run, sleep=lambda s: None)
        assert False, "expected StepError"
    except daily.StepError as e:
        assert "metadata.json" in str(e)


def test_research_fails_on_validate_error(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    (d / "metadata.json").write_text("{}")
    (d / "pillars.csv").write_text("x")
    run = FakeRun([(0, "{}"), (1, "error: row 3 bad value")])
    try:
        daily.research(tmp_path, d, run=run, sleep=lambda s: None)
        assert False, "expected StepError"
    except daily.StepError as e:
        assert "row 3" in str(e)


def test_cli_writes_error_txt(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    r = subprocess.run([sys.executable, DAILY, "--root", str(tmp_path), "render", str(d)],
                       capture_output=True, text=True, env={**os.environ, "BARRACE": "/bin/false"})
    assert r.returncode == 1
    assert (d / "error.txt").read_text().startswith("render:")


def test_upload_cli_without_token_writes_reauth(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    (d / "metadata.json").write_text("{}")
    r = subprocess.run([sys.executable, DAILY, "--root", str(tmp_path), "upload", str(d)],
                       capture_output=True, text=True,
                       env={**os.environ, "BARRACE_CONFIG": str(tmp_path / "cfg")})
    assert r.returncode == 1, r.stderr
    assert "daily.py auth" in (d / "error.txt").read_text()


def test_render_runs_from_repo_root(tmp_path):
    run = FakeRun([(0, ""), (0, "")])
    daily.render(tmp_path, tmp_path / "v", run=run)
    assert [c[1] for c in run.calls] == ["render", "thumb"]
    assert run.cwds == [tmp_path, tmp_path]


def ok_step(root, d):
    pass


def test_run_all_success(tmp_path):
    write_topics(tmp_path, [row()])
    sent = []

    def upload(root, d):
        (d / "upload.json").write_text(json.dumps({"main": "m1", "short": "s1"}))
    steps = [("research", ok_step), ("render", ok_step), ("upload", upload)]
    assert daily.run_all(tmp_path, TODAY, steps=steps, notify=sent.append) == 0
    assert daily.read_rows(tmp_path / "topics_done.csv")[0]["status"] == "uploaded"
    assert "✅" in sent[0] and "m1" in sent[0]
    assert not (tmp_path / "videos" / ".lock").exists()


def test_run_all_stops_at_failed_step(tmp_path):
    write_topics(tmp_path, [row()])
    sent, called = [], []

    def bad(root, d):
        raise daily.StepError("ffmpeg missing")
    steps = [("research", ok_step), ("render", bad), ("upload", lambda r, d: called.append(1))]
    assert daily.run_all(tmp_path, TODAY, steps=steps, notify=sent.append) == 1
    done = daily.read_rows(tmp_path / "topics_done.csv")[0]
    assert done["status"] == "failed" and "render: ffmpeg missing" in done["error"]
    assert called == []
    assert "FAILED" in sent[0]


def test_run_all_empty_list_notifies(tmp_path):
    sent = []
    assert daily.run_all(tmp_path, TODAY, steps=[], notify=sent.append) == 3
    assert "empty" in sent[0]


def test_headerless_topics_not_wiped(tmp_path):
    p = tmp_path / "topics.csv"
    p.write_text("race,A,,\nrace,B,,\npillars,C,,\n")
    sent = []
    assert daily.run_all(tmp_path, TODAY, steps=[], notify=sent.append) == 2
    assert p.read_text() == "race,A,,\nrace,B,,\npillars,C,,\n"
    assert "header" in sent[0]
    assert not (tmp_path / "videos" / ".lock").exists()


def test_excel_bom_header_ok(tmp_path):
    (tmp_path / "topics.csv").write_bytes("﻿style,title,youtube_url,comment\nrace,A,,\n".encode())
    r = row(title="A")
    del r["look"]  # an old 4-column file has no look column
    assert daily.read_rows(tmp_path / "topics.csv") == [r]


def test_retry_after_failed_upload_resumes_same_folder(tmp_path):
    write_topics(tmp_path, [row()])
    calls = []

    def research(root, d):
        calls.append("research")

    def render(root, d):
        calls.append("render")

    def upload_main_then_fail(root, d):
        (d / "upload.json").write_text(json.dumps({"main": "m1"}))
        raise daily.StepError("token expired")
    assert daily.run_all(tmp_path, TODAY, steps=[("research", research), ("render", render),
                                                 ("upload", upload_main_then_fail)],
                         notify=lambda m: None) == 1
    first = daily.read_rows(tmp_path / "topics_done.csv")[0]["folder"]

    write_topics(tmp_path, [row()])  # user puts the row back, no comment
    calls.clear()

    def upload_rest(root, d):
        assert json.loads((d / "upload.json").read_text()) == {"main": "m1"}
        (d / "upload.json").write_text(json.dumps({"main": "m1", "short": "s1"}))
    assert daily.run_all(tmp_path, TODAY, steps=[("research", research), ("render", render),
                                                 ("upload", upload_rest)],
                         notify=lambda m: None) == 0
    assert calls == []
    done = daily.read_rows(tmp_path / "topics_done.csv")
    assert done[-1]["folder"] == first and done[-1]["status"] == "uploaded"
    assert not (Path(first) / "error.txt").exists()


def redo_dir(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    (d / "topic.json").write_text(json.dumps({"row": row(comment="India missing")}))
    (d / "metadata.json").write_text("{}")
    (d / "data.csv").write_text("x")
    (d / "research_notes.md").write_text("# notes\n")
    return d


def test_redo_without_fix_notes_fails(tmp_path):
    d = redo_dir(tmp_path)
    run = FakeRun([(0, "{}"), (0, "ok")])
    try:
        daily.research(tmp_path, d, run=run, sleep=lambda s: None)
        assert False, "expected StepError"
    except daily.StepError as e:
        assert "Redo fixes" in str(e)


def test_redo_with_fix_notes_passes(tmp_path):
    d = redo_dir(tmp_path)

    def fix(cmd):
        if cmd[0].endswith("claude"):
            (d / "research_notes.md").write_text("# notes\n## Redo fixes\n- added India\n")
    daily.research(tmp_path, d, run=FakeRun([(0, "{}"), (0, "ok")], fix), sleep=lambda s: None)


def test_claude_cmd_scopes_writes_to_folder(tmp_path):
    d = tmp_path / "videos" / "x"
    cmd = daily.claude_cmd(d, tmp_path)
    allowed = cmd[cmd.index("--allowedTools") + 1:cmd.index("--disallowedTools")]
    assert "acceptEdits" not in cmd
    assert "Write" not in allowed and "Edit" not in allowed
    assert f"Write(/{d}/**)" in allowed and f"Edit(/{d}/**)" in allowed
    denied = cmd[cmd.index("--disallowedTools") + 1:cmd.index("--output-format")]
    assert any(".config/barrace" in t for t in denied)


def test_research_env_has_no_secrets(tmp_path, monkeypatch):
    monkeypatch.setenv("TELEGRAM_BOT_TOKEN", "secret")
    monkeypatch.setenv("CLAUDE_CODE_OAUTH_TOKEN", "tok")
    d = tmp_path / "v"
    d.mkdir()
    run = FakeRun([(0, "{}")])
    try:
        daily.research(tmp_path, d, run=run, sleep=lambda s: None)
    except daily.StepError:
        pass
    env = run.envs[0]
    assert "TELEGRAM_BOT_TOKEN" not in env
    assert env["CLAUDE_CODE_OAUTH_TOKEN"] == "tok"


def test_research_fails_if_tool_files_changed(tmp_path):
    (tmp_path / "automation").mkdir()
    (tmp_path / "automation" / "youtube.py").write_text("private")
    d = tmp_path / "v"
    d.mkdir()

    def tamper(cmd):
        if cmd[0].endswith("claude"):
            (tmp_path / "automation" / "youtube.py").write_text("public")
            (d / "metadata.json").write_text("{}")
            (d / "data.csv").write_text("x")
    try:
        daily.research(tmp_path, d, run=FakeRun([(0, "{}"), (0, "ok")], tamper),
                       sleep=lambda s: None)
        assert False, "expected StepError"
    except daily.StepError as e:
        assert "automation/youtube.py" in str(e)


def test_research_time_limit_20_min():
    assert daily.RESEARCH_TIMEOUT_S == 20 * 60


def test_claude_cmd_ignores_user_hooks(tmp_path):
    # user-level hooks (e.g. command rewriters) break the tool allowlist
    cmd = daily.claude_cmd(tmp_path / "videos" / "x", tmp_path)
    assert cmd[cmd.index("--setting-sources") + 1] == "project"


def test_old_four_column_topics_still_read(tmp_path):
    (tmp_path / "topics.csv").write_text("style,title,youtube_url,comment\nstocks,Apple vs Nvidia,,\n")
    r = cli(tmp_path, "pick")
    assert r.returncode == 0, r.stdout + r.stderr


def test_look_validated():
    assert daily.row_problem(row(style="stocks", title="A vs B", look="neon")) is None
    assert daily.row_problem(row(style="stocks", title="A vs B", look="NEON")) is None
    assert "only for stocks" in daily.row_problem(row(look="neon"))
    assert "look must be" in daily.row_problem(row(style="stocks", title="A vs B", look="pink"))


def test_look_lowercased_into_topic_json(tmp_path):
    write_topics(tmp_path, [row(style="stocks", title="Apple vs Nvidia", look="Neon")])
    d = daily.pick(tmp_path, TODAY)["dir"]
    assert json.loads((Path(d) / "topic.json").read_text())["row"]["look"] == "neon"


def test_mixed_case_look_row_still_leaves_topics(tmp_path):
    write_topics(tmp_path, [row(style="stocks", title="Apple vs Nvidia", look="Neon"), row(title="Next")])
    d = Path(daily.pick(tmp_path, TODAY)["dir"])
    daily.finish(tmp_path, d, "failed")
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == ["Next"]
