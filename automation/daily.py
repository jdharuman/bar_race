"""Daily video pipeline steps, called in order by n8n (see docs/automation.md).

pick -> research <dir> -> render <dir> -> upload <dir> -> finish <dir> --status ...
Each step prints one JSON line. On failure a step writes <dir>/error.txt and exits 1.
"""
import argparse
import contextlib
import csv
import fcntl
import json
import os
import subprocess
import re
import shutil
import sys
import time
from datetime import date, datetime
from pathlib import Path
from zoneinfo import ZoneInfo
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent.parent
TOPIC_COLS = ["style", "title", "youtube_url", "comment", "look", "theme"]  # last two optional
DONE_COLS = TOPIC_COLS + ["finished_at", "folder", "status", "main_video_id",
                          "short_video_id", "error"]
STYLES = ("race", "pillars", "stocks", "compare", "quiz")
LINE_STYLES = ("stocks", "compare")  # rendered as line charts
LOOKS = ("", "light", "neon")  # neon = black background, glowing lines (line charts only)
# Weekly rhythm: each weekday has a theme (also the playlist the videos belong in).
THEMES = {"sports": "Sports", "india": "India & Tamil Nadu", "economy": "Economy & Stocks",
          "earth": "Earth & Space", "tech": "Tech & Cars", "history": "History & Countries",
          "people": "People & World"}
WEEK = ["sports", "india", "economy", "earth", "tech", "history", "people"]  # Mon..Sun
# One video per graph slot per night, in this order; then quiz Shorts fill whatever
# YouTube quota is left (see run_all). The quiz slot can repeat; the others run once.
SLOTS = {"race": ("race",), "pillars": ("pillars",), "lines": LINE_STYLES, "quiz": ("quiz",)}
GRAPH_SLOTS = ["race", "pillars"]  # lines (stocks/compare) Shorts drew the fewest views


def graph_slots() -> list[str]:
    """Nightly graph slots: BARRACE_SLOTS (e.g. "race,pillars,lines"), else GRAPH_SLOTS."""
    env = [x.strip().lower() for x in os.environ.get("BARRACE_SLOTS", "").split(",") if x.strip()]
    return [x for x in env if x in SLOTS and x != "quiz"] or list(GRAPH_SLOTS)
QUIZ_MAX_PER_NIGHT = 40  # safety cap; the quota is the real limit


def main_on() -> bool:
    """16:9 main videos (+ thumbnail) are off: only Shorts are made and uploaded.
    BARRACE_MAIN=on in ~/.config/barrace/env brings the main video back."""
    return os.environ.get("BARRACE_MAIN", "").lower() == "on"


def formats(d: Path) -> tuple[str, ...]:
    """What gets rendered and uploaded for a video folder."""
    return ("main", "short") if main_on() and not is_quiz(d) else ("short",)


def theme_of(day: date) -> str:
    return WEEK[day.weekday()]
LOCK_STALE_S = 6 * 3600


class StepError(Exception):
    pass


NO_DATA = ".no-data"  # marker: research found no trustworthy source and stopped on purpose


class BadFile(Exception):
    pass


def read_rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    with path.open(newline="", encoding="utf-8-sig") as f:  # -sig: Excel's BOM
        reader = csv.DictReader(f)
        missing = [c for c in TOPIC_COLS[:3] if c not in (reader.fieldnames or [])]
        if missing:
            raise BadFile(f"{path.name}: header row must be {','.join(TOPIC_COLS)} "
                          f"(missing {', '.join(missing)}); file left unchanged")
        return [{k: (v or "").strip() for k, v in r.items() if k} for r in reader
                if any((v or "").strip() for v in r.values())]


def write_rows(path: Path, cols: list[str], rows: list[dict]) -> None:
    tmp = path.with_suffix(".tmp")
    with tmp.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=cols, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)
    tmp.replace(path)


def row_problem(row: dict) -> str | None:
    if row.get("style") not in STYLES:
        return f"style must be one of {', '.join(STYLES)} (got {row.get('style')!r})"
    look = row.get("look", "").lower()
    if look not in LOOKS:
        return f"look must be empty, light or neon (got {row.get('look')!r})"
    if look == "neon" and row.get("style") not in LINE_STYLES:
        return "look = neon is only for line charts (stocks, compare)"
    theme = row.get("theme", "").lower()
    if theme and theme not in THEMES:
        return f"theme must be empty or one of {', '.join(THEMES)} (got {row.get('theme')!r})"
    if not row.get("title") and not row.get("youtube_url"):
        return "need a title or a youtube_url"
    if row.get("style") == "quiz":
        import quiz
        try:
            quiz.parse_title(row.get("title", ""))
        except quiz.QuizError as e:
            return str(e)
    if row.get("youtube_url") and not video_id(row["youtube_url"]):
        return f"not a YouTube video link: {row['youtube_url']}"
    return None


def video_id(url: str) -> str | None:
    if not url:
        return None
    u = urlparse(url)
    if u.hostname == "youtu.be":
        return u.path.strip("/") or None
    if u.path.startswith("/shorts/"):
        return u.path.split("/")[2] or None
    return parse_qs(u.query).get("v", [None])[0]


def slugify(row: dict) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", row.get("title", "").lower()).strip("-")[:50].strip("-")
    return s or f"yt-{video_id(row.get('youtube_url', ''))}"


def _norm(s: str) -> str:
    return " ".join(s.lower().split())


def find_previous(row: dict, done_rows: list[dict], root: Path, status: str | None = None) -> Path | None:
    vid = video_id(row.get("youtube_url", ""))
    for d in reversed(done_rows):
        if status and d.get("status") != status:
            continue
        same = (row.get("title") and _norm(d.get("title", "")) == _norm(row["title"])) or \
               (vid and video_id(d.get("youtube_url", "")) == vid)
        folder = Path(d.get("folder", "")) if d.get("folder") else None
        if d.get("style") == row["style"] and same and folder:
            folder = folder if folder.is_absolute() else root / folder
            if folder.is_dir():
                return folder
    return None


def _lock(root: Path) -> Path:
    return root / "videos" / ".lock"


def take_lock(root: Path, kind: str = "daily", title: str = "") -> bool:
    """One run at a time (daily or Telegram request). The lock says who holds it."""
    lock = _lock(root)
    lock.parent.mkdir(parents=True, exist_ok=True)
    if lock.exists() and time.time() - lock.stat().st_mtime < LOCK_STALE_S:
        return False
    lock.write_text(json.dumps({"kind": kind, "title": title,
                                "since": datetime.now().isoformat(timespec="seconds")}))
    return True


def lock_holder(root: Path) -> dict | None:
    """Who holds a live lock: {"kind", "title", "since"}, or None when free/stale."""
    lock = _lock(root)
    try:
        if time.time() - lock.stat().st_mtime >= LOCK_STALE_S:
            return None
        text = lock.read_text()
    except FileNotFoundError:
        return None
    try:
        info = json.loads(text)
        return info if isinstance(info, dict) else {"kind": "daily", "since": text.strip()}
    except ValueError:  # older plain-timestamp lock
        return {"kind": "daily", "title": "", "since": text.strip()}


def release_lock(root: Path) -> None:
    _lock(root).unlink(missing_ok=True)


@contextlib.contextmanager
def topics_lock(root: Path):
    """Serialise read-modify-write of topics.csv (nightly run vs. bot approvals)."""
    path = root / "videos" / ".topics.lock"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(f, fcntl.LOCK_UN)


def insert_topic(root: Path, row: dict) -> None:
    """Put `row` at the top of topics.csv (it then runs on its theme's next day)."""
    with topics_lock(root):
        topics = read_rows(root / "topics.csv")
        write_rows(root / "topics.csv", TOPIC_COLS, [{c: row.get(c, "") for c in TOPIC_COLS}, *topics])


def move_to_done(root: Path, row: dict, **fields) -> None:
    with topics_lock(root):
        _move_to_done(root, row, **fields)


def _move_to_done(root: Path, row: dict, **fields) -> None:
    """Remove the first row equal to `row` from topics.csv and append it to topics_done.csv."""
    topics = read_rows(root / "topics.csv")
    def key(r: dict) -> list[str]:  # look/theme are matched case-blind ("Neon" == "neon")
        return [r.get(c, "").lower() if c in ("style", "look", "theme") else r.get(c, "") for c in TOPIC_COLS]
    for i, r in enumerate(topics):
        if key(r) == key(row):
            del topics[i]
            break
    write_rows(root / "topics.csv", TOPIC_COLS, topics)
    done = read_rows(root / "topics_done.csv")
    done.append({**row, "finished_at": datetime.now().isoformat(timespec="seconds"), **fields})
    write_rows(root / "topics_done.csv", DONE_COLS, done)


class Invalid(Exception):
    pass


def pinned(row: dict) -> bool:
    """A row whose comment starts with "pin:" (e.g. a test week) runs before other rows
    of its theme, whatever growth reviews or the scout move to the top of the file."""
    return row.get("comment", "").strip().lower().startswith("pin:")


def choose(topics: list[dict], styles=None, theme: str | None = None) -> int | None:
    """Index of the topic to make: among `styles` (None = any), first today's theme,
    then a row without theme, then any; pinned rows (see `pinned`) before the rest
    each time. None = no topic of these styles."""
    cands = [i for i, r in enumerate(topics) if styles is None or r.get("style", "").lower() in styles]
    if not cands:
        return None
    cands.sort(key=lambda i: not pinned(topics[i]))  # stable: file order otherwise
    if theme:
        for want in (theme, ""):
            for i in cands:
                if topics[i].get("theme", "").lower() == want:
                    return i
    return cands[0]


def pick(root: Path, today: date, styles=None, theme: str | None = None) -> dict:
    """Start a run on the chosen topic (default: the first). Caller must hold the lock."""
    topics = read_rows(root / "topics.csv")
    i = choose(topics, styles, theme)
    if i is None:
        raise LookupError("empty")
    row = {c: topics[i].get(c, "") for c in TOPIC_COLS}
    row["style"], row["look"], row["theme"] = row["style"].lower(), row["look"].lower(), row["theme"].lower()
    if problem := row_problem(row):
        move_to_done(root, row, status="failed", error=f"pick: {problem}")
        raise Invalid(problem)
    if pinned(row):  # "pin: ..." only schedules the row; it is not a redo request
        row["comment"] = ""
    done = read_rows(root / "topics_done.csv")
    if not row["comment"]:
        # retry of a failed run: continue in its folder; finished steps are skipped
        prev = find_previous(row, done, root, status="failed")
        if prev:
            (prev / "error.txt").unlink(missing_ok=True)
            topic = json.loads((prev / "topic.json").read_text())
            topic["resumed_at"] = datetime.now().isoformat(timespec="seconds")
            (prev / "topic.json").write_text(json.dumps(topic, indent=2))
            return {"dir": str(prev), "style": row["style"], "title": row["title"],
                    "youtube_url": row["youtube_url"], "redo_of": None, "resumed": True}
    base = root / "videos" / f"{today.isoformat()}-{slugify(row)}"
    d, n = base, 2
    while d.exists():
        d, n = base.with_name(f"{base.name}-{n}"), n + 1
    prev = find_previous(row, done, root) if row["comment"] else None
    if prev:
        shutil.copytree(prev, d, ignore=shutil.ignore_patterns(
            "out", "upload.json", "error.txt", "topic.json", ".ok-*", "research_notes.prev.md"))
        if (d / "research_notes.md").exists():  # fresh notes prove the redo was worked on
            (d / "research_notes.md").rename(d / "research_notes.prev.md")
    else:
        d.mkdir(parents=True)
    topic = {"row": row, "redo_of": str(prev) if prev else None,
             "picked_at": datetime.now().isoformat(timespec="seconds")}
    (d / "topic.json").write_text(json.dumps(topic, indent=2))
    return {"dir": str(d), "style": row["style"], "title": row["title"],
            "youtube_url": row["youtube_url"], "redo_of": topic["redo_of"]}


RESEARCH_TIMEOUT_S = 20 * 60
LIMIT_RETRY_S = 30 * 60


def barrace_bin(root: Path) -> str:
    return os.environ.get("BARRACE", str(root / "target" / "release" / "barrace"))


def claude_cmd(d: Path, root: Path) -> list[str]:
    bb = barrace_bin(root)
    prompt = (
        f"Read .claude/skills/barrace-research/SKILL.md and follow it exactly to research "
        f"the topic in {d}/topic.json and write every output file into {d}. "
        f"The barrace binary is {bb}. Finish only when `{bb} validate {d}` reports no errors."
    )
    tools = ["Read", f"Write(/{d}/**)", f"Edit(/{d}/**)", "Glob", "Grep", "WebSearch",
             "WebFetch", "Bash(curl:*)", "Bash(uv run:*)", "Bash(yt-dlp:*)",
             f"Bash({bb} validate:*)", "Bash(ls:*)", "Bash(mkdir:*)", "Bash(unzip:*)",
             "Bash(file:*)", "Bash(head:*)", "Bash(wc:*)"]
    denied = ["Read(~/.config/barrace/**)", "Read(~/.ssh/**)", "Bash(yt-dlp --exec:*)"]
    return [os.environ.get("CLAUDE_BIN", "claude"), "-p", prompt, "--model", "sonnet", "--setting-sources", "project",
            "--allowedTools", *tools, "--disallowedTools", *denied,
            "--output-format", "json"]


def _rephrase(notes: Path, limit: int = 600) -> str:
    """The "How to rephrase" section of research_notes.md (written when research gives
    up), so Telegram shows what to ask for instead."""
    try:
        text = notes.read_text()
    except OSError:
        return ""
    m = re.search(r"^#+[^\n]*rephras[^\n]*\n(.*?)(?=^#|\Z)", text, re.I | re.M | re.S)
    if not m:
        return ""
    body = m.group(1).strip()
    return body if len(body) <= limit else body[:limit].rsplit("\n", 1)[0] + "\n…"


def _claude_result(stdout: str, limit: int = 700) -> str:
    """The agent's final message from `claude --output-format json`, shortened."""
    try:
        text = json.loads(stdout or "").get("result") or ""
    except (ValueError, AttributeError):
        text = ""
    text = text.strip() or "(no final message)"
    return text if len(text) <= limit else text[:limit].rsplit(" ", 1)[0] + " …"


def _tail(text: str, n: int = 30) -> str:
    return "\n".join((text or "").strip().splitlines()[-n:])


# The agent may change only its video folder. Anything here changing fails the run.
GUARDED = ["automation", "src", ".claude", "n8n", "music", "CLAUDE.md", "channel.toml",
           "Cargo.toml"]
AGENT_ENV = ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TZ", "CLAUDE_CODE_OAUTH_TOKEN",
             "CLAUDE_BIN", "BARRACE"]


def snapshot(root: Path) -> dict:
    files = {}
    for g in GUARDED:
        p = root / g
        for f in ([p] if p.is_file() else p.rglob("*") if p.is_dir() else []):
            if f.is_file() and "__pycache__" not in f.parts and ".venv" not in f.parts:
                st = f.stat()
                files[str(f.relative_to(root))] = (st.st_mtime_ns, st.st_size)
    return files


def is_quiz(d: Path) -> bool:
    try:
        return json.loads((d / "topic.json").read_text())["row"].get("style") == "quiz"
    except (FileNotFoundError, ValueError, KeyError):
        return False


def make_quiz(root: Path, d: Path, run=subprocess.run) -> None:
    """Quiz rows skip Claude: the questions come from bundled, sourced data."""
    import quiz
    title = json.loads((d / "topic.json").read_text())["row"]["title"]
    try:
        quiz.generate(root, d, title)
    except quiz.QuizError as e:
        raise StepError(str(e)) from e
    v = run([barrace_bin(root), "validate", str(d)], cwd=root, capture_output=True, text=True)
    if v.returncode != 0:
        raise StepError(f"barrace validate failed\n{_tail(v.stdout + (v.stderr or ''))}")


def research(root: Path, d: Path, run=subprocess.run, sleep=time.sleep) -> None:
    if is_quiz(d):
        return make_quiz(root, d, run)
    cmd = claude_cmd(d, root)
    (d / NO_DATA).unlink(missing_ok=True)
    # ponytail: env scrub + path-scoped writes + tripwire; `uv run` can still run any
    # Python as this user. Real boundary = Claude Code sandbox or a separate unix user.
    env = {k: os.environ[k] for k in AGENT_ENV if k in os.environ}
    before = snapshot(root)
    for attempt in (1, 2):
        r = run(cmd, cwd=root, env=env, capture_output=True, text=True,
                timeout=RESEARCH_TIMEOUT_S)
        out = f"{r.stdout or ''}\n{r.stderr or ''}"
        (d / "research.log").write_text(out)
        limited = "usage limit" in out.lower() or "limit reached" in out.lower()
        if limited and attempt == 1:
            sleep(LIMIT_RETRY_S)
            continue
        if r.returncode != 0:
            raise StepError(f"claude exited {r.returncode}\n{_tail(out)}")
        break
    after = snapshot(root)
    changed = sorted(k for k in before.keys() | after.keys() if before.get(k) != after.get(k))
    if changed:
        raise StepError("research changed tool files outside the video folder: "
                        + ", ".join(changed[:10]) + " (check with git diff before running again)")
    if not (d / "metadata.json").exists():
        (d / NO_DATA).touch()
        tips = _rephrase(d / "research_notes.md")
        raise StepError("research gave up, no video made (no trustworthy data found). "
                        f"Claude said:\n{_claude_result(r.stdout, 450 if tips else 700)}\n"
                        + (f"\n💡 Try instead:\n{tips}\n" if tips else "")
                        + "Full reasons: research_notes.md in the video folder.")
    topic = json.loads((d / "topic.json").read_text()) if (d / "topic.json").exists() else {}
    notes = d / "research_notes.md"
    redo = topic.get("row", {}).get("comment") and (topic.get("redo_of") or not topic.get("request"))
    if redo and (not notes.exists() or "redo fixes" not in notes.read_text().lower()):
        raise StepError("redo: research_notes.md has no 'Redo fixes' section, "
                        "so the comment was not worked on")
    if not ((d / "data.csv").exists() or (d / "pillars.csv").exists()):
        raise StepError("claude finished but wrote no data.csv or pillars.csv")
    v = run([barrace_bin(root), "validate", str(d)], cwd=root, capture_output=True, text=True)
    if v.returncode != 0:
        raise StepError(f"barrace validate failed\n{_tail(v.stdout + (v.stderr or ''))}")


def render(root: Path, d: Path, run=subprocess.run) -> None:
    bb = barrace_bin(root)
    cmds = ([[bb, "render", str(d), "--format", "short"]] if formats(d) == ("short",) else
            [[bb, "render", str(d), "--format", "both"], [bb, "thumb", str(d)]])
    for cmd in cmds:
        r = run(cmd, cwd=root, capture_output=True, text=True)
        if r.returncode != 0:
            raise StepError(f"{' '.join(cmd[1:2])} exited {r.returncode}\n"
                            f"{_tail((r.stdout or '') + (r.stderr or ''))}")


def run_step(name: str, d: Path, fn) -> int:
    """Run a step; on failure write <d>/error.txt and return 1."""
    try:
        fn()
    except Exception as e:  # any crash must reach error.txt and the Telegram report
        if isinstance(e, subprocess.TimeoutExpired):
            e = f"timed out after {e.timeout:.0f} s"
        elif type(e).__name__ != "StepError":
            e = f"{type(e).__name__}: {e}"
        (d / "error.txt").write_text(f"{name}: {e}\n")
        _print({"ok": False, "error": f"{name}: {str(e).splitlines()[0]}"})
        return 1
    _print({"ok": True})
    return 0


def studio(vid: str) -> str:
    return f"https://studio.youtube.com/video/{vid}/edit"


def finish(root: Path, d: Path, status: str) -> dict:
    """Move the picked row to topics_done.csv, release the lock, build the report."""
    row = json.loads((d / "topic.json").read_text())["row"]
    up = json.loads((d / "upload.json").read_text()) if (d / "upload.json").exists() else {}
    err = (d / "error.txt").read_text().strip() if (d / "error.txt").exists() else ""
    if status == "failed" and not err:
        err = "unknown error (see the n8n execution log)"
    if status == "waiting":
        err = ""
    move_to_done(root, row, folder=str(d), status=status, error=err,
                 main_video_id=up.get("main", ""), short_video_id=up.get("short", ""))
    release_lock(root)
    return report(row, up, status, err, "To retry, put the row back at the top of topics.csv.")


def report(row: dict, up: dict, status: str, err: str, retry_hint: str, tag: str = "") -> dict:
    name = row["title"] or row["youtube_url"]
    links = {k: studio(up[k]) for k in ("main", "short") if up.get(k)}
    playlist = THEMES.get(row.get("theme", "").lower())
    if status == "uploaded" and not links.get("main"):  # Short only
        msg = (f"✅ Ready to review{tag}: {name}\nShort: {links.get('short', '-')}\n"
               "Watch it, then publish in YouTube Studio.")
        if playlist and row.get("style") != "quiz":
            msg += f"\nPlaylist: {playlist}"
    elif status == "uploaded":
        msg = (f"✅ Ready to review{tag}: {name}\nMain: {links.get('main', '-')}\n"
               f"Short: {links.get('short', '-')}\nWatch both, then publish in YouTube Studio.")
        if playlist:
            msg += f"\nPlaylist: {playlist}"
    elif status == "waiting":
        msg = (f"⏳ Made, upload waiting for YouTube quota{tag}: {name}\n"
               "It uploads automatically in the next run after the quota resets "
               "(midnight Pacific time).")
        if links:
            msg += "\nAlready uploaded (private): " + " ".join(links.values())
    else:
        msg = f"❌ FAILED{tag}: {name}\n{err[-1500:]}\n{retry_hint}"
        if links:
            msg += "\nPartly uploaded (private): " + " ".join(links.values())
    return {"status": status, "title": name, "main_url": links.get("main", ""),
            "short_url": links.get("short", ""), "error": err, "message": msg}


def start(root: Path, today: date, slot: str | None = None) -> tuple[int, dict]:
    """Lock + pick. Exit codes: 0 picked, 2 invalid row, 3 empty list/slot, 4 locked.
    `slot` (race/pillars/lines) picks that chart type, preferring today's theme."""
    try:
        topics = read_rows(root / "topics.csv")
        read_rows(root / "topics_done.csv")
    except BadFile as e:
        return 2, {"invalid": str(e), "message": f"❌ barrace did nothing today: {e}"}
    if not topics:
        return 3, {"empty": True, "message": "barrace: topics.csv is empty, nothing to make today."}
    styles, theme = (SLOTS[slot], theme_of(today)) if slot else (None, None)
    i = choose(topics, styles, theme)
    if i is None:
        return 3, {"empty": True, "slot": slot}
    if not take_lock(root, "daily", topics[i].get("title", "")):
        return 4, {"locked": True, "holder": lock_holder(root)}
    try:
        return 0, pick(root, today, styles, theme)
    except Invalid as e:
        release_lock(root)
        return 2, {"invalid": str(e), "message": f"❌ barrace skipped a bad topics.csv row: {e}"}


def _upload(root: Path, d: Path) -> None:
    import youtube
    youtube.upload(d, root)


STEPS = [("research", research), ("render", render), ("upload", _upload)]


def send_telegram(text: str) -> None:
    """Send `text` to TELEGRAM_CHAT_ID; prints instead when not configured. Never raises."""
    token, chat = os.environ.get("TELEGRAM_BOT_TOKEN"), os.environ.get("TELEGRAM_CHAT_ID")
    print(text)
    if not (token and chat):
        return
    import urllib.parse
    import urllib.request
    data = urllib.parse.urlencode({"chat_id": chat, "text": text}).encode()
    try:
        urllib.request.urlopen(f"https://api.telegram.org/bot{token}/sendMessage", data, timeout=30)
    except Exception as e:  # a lost message must not fail the run
        print(f"telegram send failed: {e}", file=sys.stderr)


STATE_FILES = ["topics.csv", "topics_done.csv", "requests_done.csv", "videos/growth/dropped.csv"]


def state_repo() -> Path:
    return Path(os.environ.get("BARRACE_STATE") or Path.home() / "barrace-state")


def commit_state(root: Path, message: str) -> bool:
    """Copy the topic lists into the local state repo (~/barrace-state, its own git
    repo; never pushed, never the code repo) and commit when anything changed.
    Returns True when a commit was made. Never raises."""
    repo = state_repo()

    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True,
                              timeout=60)
    try:
        if not (repo / ".git").exists():
            repo.mkdir(parents=True, exist_ok=True)
            git("init", "-q")
            git("config", "user.name", "barrace")
            git("config", "user.email", "barrace@localhost")
        for name in STATE_FILES:
            src = root / name
            if src.exists():
                dest = repo / Path(name).name
                shutil.copyfile(src, dest)
                git("add", dest.name)
        if git("diff", "--cached", "--quiet").returncode == 0:
            return False
        r = git("commit", "-q", "-m", message)
        if r.returncode:
            print(f"state commit failed: {r.stderr.strip()}", file=sys.stderr)
        return r.returncode == 0
    except Exception as e:  # history is a convenience; it must never fail a run
        print(f"state commit failed: {e}", file=sys.stderr)
        return False


TG_VIDEO_MAX = 49 * 1024 * 1024  # bot API upload limit is 50 MB


def multipart(fields: dict, file_field: str, path: Path) -> tuple[bytes, str]:
    """multipart/form-data body (fields + one mp4 file) for Telegram uploads."""
    boundary = f"barrace{os.urandom(8).hex()}"
    parts = []
    for k, v in fields.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{file_field}"; '
                 f'filename="{path.name}"\r\nContent-Type: video/mp4\r\n\r\n'.encode())
    parts.append(path.read_bytes())
    parts.append(f"\r\n--{boundary}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={boundary}"


def telegram_video(token: str, chat, path: Path, caption: str) -> None:
    """Upload one mp4 to a Telegram chat (raises on failure)."""
    import urllib.request
    if path.stat().st_size > TG_VIDEO_MAX:
        raise RuntimeError(f"{path.name} is over Telegram's 50 MB bot limit")
    body, ctype = multipart({"chat_id": chat, "caption": caption[:1000],
                             "supports_streaming": "true"}, "video", path)
    req = urllib.request.Request(f"https://api.telegram.org/bot{token}/sendVideo", body,
                                 {"Content-Type": ctype})
    with urllib.request.urlopen(req, timeout=300) as r:
        if not json.loads(r.read()).get("ok"):
            raise RuntimeError("sendVideo was refused")


def send_telegram_video(path: Path, caption: str) -> None:
    """Send a Short to TELEGRAM_CHAT_ID for review on the phone. Never raises."""
    token, chat = os.environ.get("TELEGRAM_BOT_TOKEN"), os.environ.get("TELEGRAM_CHAT_ID")
    if not (token and chat and path.exists()):
        return
    try:
        telegram_video(token, chat, path, caption)
    except Exception as e:  # a lost preview must not fail the run
        print(f"telegram video send failed: {e}", file=sys.stderr)


def send_preview(d: Path, send_video) -> None:
    """Graph Shorts go to Telegram automatically; quiz Shorts only via the bot's /preview."""
    if is_quiz(d):
        return
    try:
        title = json.loads((d / "topic.json").read_text())["row"].get("title", "") or d.name
    except (OSError, ValueError, KeyError):
        title = d.name
    send_video(d / "out" / "short.mp4", f"🎬 {title} (Short preview)")


LOCK_POLL_S = 60
DAILY_WAIT_S = 4 * 3600    # the daily video waits this long for a Telegram request to finish
REQUEST_WAIT_S = 6 * 3600  # a Telegram request waits this long for a running video


def run_all(root: Path, today: date, steps=None, notify=send_telegram,
            sleep=time.sleep, wait_s: int = DAILY_WAIT_S, slots=None) -> int:
    """The nightly run: first held uploads, then one video pair per slot.
    Exit codes: 0 ok (incl. skipped slots), 1 a step failed, 2 bad topics, 3 empty, 4 locked."""
    steps = STEPS if steps is None else steps
    try:
        topics = read_rows(root / "topics.csv")
    except BadFile as e:
        notify(f"❌ barrace did nothing today: {e}")
        return 2
    if not topics:
        notify("barrace: topics.csv is empty, nothing to make today.")
        return 3
    reject_bad_rows(root, notify)
    upload_fn = dict(steps).get("upload")
    if upload_fn:
        upload_waiting(root, upload_fn, notify)
    codes, skipped = [], []
    fill_quiz = slots is None and quiz_on()
    if slots is None:
        slots = graph_slots()
    for slot in slots:
        code = run_slot(root, today, slot, steps, notify, sleep, wait_s)
        if code == 3:
            skipped.append(slot)
        else:
            codes.append(code)
    if fill_quiz:
        made = 0
        while made < QUIZ_MAX_PER_NIGHT and Quota(root).left() >= COST_VIDEO:
            top_up_quiz(root)
            code = run_slot(root, today, "quiz", steps, notify, sleep, wait_s)
            codes.append(code)
            if code != 0:  # a failing quiz would fail the same way again tonight
                break
            made += 1
    if skipped and codes:
        notify(f"barrace: no {' / '.join(skipped)} topic in topics.csv "
               f"(today's theme: {THEMES[theme_of(today)]}); add some to fill those slots.")
    if not codes:
        notify("barrace: no topic for any slot in topics.csv, nothing made today.")
        return 3
    for c in (1, 2, 4):
        if c in codes:
            return c
    return 0


def plan_night(topics: list[dict], done_titles: list[str], night: date, left: int) -> list:
    """What the run on `night` would make, as (slot, row) pairs (row None: slot empty),
    if every video works. Mirrors run_all: graph slots, then quizzes while `left`
    quota units last. Generated quiz rows have "generated": True."""
    import quiz
    rows, plan, theme = list(topics), [], theme_of(night)
    per_video = COST_VIDEO * 2 + COST_THUMB if main_on() else COST_VIDEO
    for slot in graph_slots():
        i = choose(rows, SLOTS[slot], theme)
        plan.append((slot, None if i is None else rows.pop(i)))
        if i is not None:
            left -= per_video
    if quiz_on():
        quiz_titles = list(done_titles)  # oldest first, like top_up_quiz
        quiz_titles += [r.get("title", "") for r in topics if r.get("style", "").lower() == "quiz"]
        n = 0
        while left >= COST_VIDEO and n < QUIZ_MAX_PER_NIGHT:
            i = choose(rows, ("quiz",), theme)
            if i is None:
                row = {"style": "quiz", "title": quiz.next_title(quiz_titles), "generated": True}
                quiz_titles.append(row["title"])
            else:
                row = rows.pop(i)
            plan.append(("quiz", row))
            left -= COST_VIDEO
            n += 1
    return plan


def quiz_on() -> bool:
    """Quiz Shorts fill the quota left after the graph slots; BARRACE_QUIZ=off stops them."""
    return os.environ.get("BARRACE_QUIZ", "").lower() != "off"


def top_up_quiz(root: Path) -> None:
    """Make sure topics.csv has a quiz row: append the next generated title if not."""
    import quiz
    topics = read_rows(root / "topics.csv")
    if any(r.get("style", "").lower() == "quiz" for r in topics):
        return
    try:
        done = read_rows(root / "topics_done.csv")
    except BadFile:
        done = []
    title = quiz.next_title([r.get("title", "") for r in done + topics  # oldest first
                             if r.get("style", "").lower() == "quiz"])
    topics.append({"style": "quiz", "title": title})
    write_rows(root / "topics.csv", TOPIC_COLS, topics)


def reject_bad_rows(root: Path, notify) -> None:
    """Move unusable topics.csv rows to topics_done.csv (failed) so none sits there unseen."""
    for r in read_rows(root / "topics.csv"):
        row = {c: r.get(c, "") for c in TOPIC_COLS}
        if problem := row_problem({**row, "style": row["style"].lower()}):
            move_to_done(root, row, status="failed", error=f"pick: {problem}")
            notify(f"❌ barrace skipped a bad topics.csv row ({row['title'] or row['youtube_url']}): {problem}")


SLOT_TRIES = 3  # a slot whose topic has no trustworthy data tries up to 2 more topics


def run_slot(root: Path, today: date, slot: str, steps, notify, sleep, wait_s: int) -> int:
    for attempt in range(1, SLOT_TRIES + 1):
        code, d = _run_slot_once(root, today, slot, steps, notify, sleep, wait_s)
        if not (code == 1 and d and (d / NO_DATA).exists()) or attempt == SLOT_TRIES:
            return code
        notify(f"↪️ barrace: trying the next {slot} topic instead.")
    return code


def _run_slot_once(root: Path, today: date, slot: str, steps, notify, sleep,
                   wait_s: int) -> tuple[int, Path | None]:
    waited = 0
    while True:
        code, payload = start(root, today, slot)
        busy_with_request = code == 4 and (payload.get("holder") or {}).get("kind") == "request"
        if not busy_with_request or waited >= wait_s:
            break
        sleep(LOCK_POLL_S)  # a Telegram request is running: wait, don't skip today's video
        waited += LOCK_POLL_S
    if code:
        if "message" in payload:
            notify(payload["message"])
        elif busy_with_request:
            notify(f"❌ barrace skipped today's video: a Telegram request was still "
                   f"running after {wait_s // 3600} h.")
        return code, None
    d = Path(payload["dir"])
    return run_steps(root, d, steps, notify, lambda status: finish(root, d, status)), d


def run_steps(root: Path, d: Path, steps, notify, finish_fn, send_video=None) -> int:
    send_video = send_video or send_telegram_video
    for name, fn in steps:
        marker = d / f".ok-{name}"
        if marker.exists():  # finished in an earlier, failed run of this folder
            continue
        (d / QUOTA_WAIT).unlink(missing_ok=True)
        if run_step(name, d, lambda: fn(root, d)):
            if (d / QUOTA_WAIT).exists():  # not a failure: the upload is held for quota
                (d / "error.txt").unlink(missing_ok=True)
                notify(finish_fn("waiting")["message"])
                send_preview(d, send_video)
                return 0
            notify(finish_fn("failed")["message"])
            return 1
        marker.touch()
    notify(finish_fn("uploaded")["message"])
    send_preview(d, send_video)
    return 0


# ---- YouTube quota (10,000 units/day per API project, reset at midnight Pacific) ----

QUOTA_WAIT = ".quota-wait"   # marker: the upload step stopped for quota, not an error
COST_VIDEO, COST_THUMB = 1600, 50


def quota_day(now: datetime | None = None) -> str:
    return (now or datetime.now(ZoneInfo("America/Los_Angeles"))).astimezone(
        ZoneInfo("America/Los_Angeles")).date().isoformat()


class Quota:
    """Units spent today, in videos/.quota.json. Limit: BARRACE_QUOTA (default 10,000);
    raise it when Google grants more."""

    def __init__(self, root: Path, limit: int | None = None, day: str | None = None):
        self.path = root / "videos" / ".quota.json"
        self.limit = limit if limit is not None else int(os.environ.get("BARRACE_QUOTA", "10000"))
        self.day = day or quota_day()
        try:
            st = json.loads(self.path.read_text())
        except (FileNotFoundError, ValueError):
            st = {}
        self.used = st.get("used", 0) if st.get("day") == self.day else 0

    def left(self) -> int:
        return self.limit - self.used

    def spend(self, units: int) -> None:
        self.used += units
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(json.dumps({"day": self.day, "used": self.used}))


def append_done(root: Path, log: str, row: dict, **fields) -> None:
    done = read_rows(root / log)
    done.append({**row, "finished_at": datetime.now().isoformat(timespec="seconds"), **fields})
    write_rows(root / log, DONE_COLS, done)


def upload_waiting(root: Path, upload_fn, notify) -> None:
    """Upload videos held for quota on an earlier day (oldest first)."""
    waiting = []
    for log in ("topics_done.csv", REQUESTS_DONE):
        try:
            rows = read_rows(root / log)
        except BadFile:
            continue
        latest = {}
        for r in rows:
            latest[r.get("folder", "")] = r
        waiting += [(log, r) for f, r in latest.items() if f and r.get("status") == "waiting"]
    if not waiting or not take_lock(root, "daily", "held uploads"):
        return
    try:
        for log, r in waiting:
            d = Path(r["folder"])
            if not (d / "topic.json").exists():
                continue
            row = json.loads((d / "topic.json").read_text())["row"]
            (d / QUOTA_WAIT).unlink(missing_ok=True)
            tag = " (Telegram request)" if log == REQUESTS_DONE else ""
            if run_step("upload", d, lambda: upload_fn(root, d)):
                if (d / QUOTA_WAIT).exists():
                    (d / "error.txt").unlink(missing_ok=True)
                    return  # still no quota: the rest waits too
                err = (d / "error.txt").read_text().strip()
                append_done(root, log, row, folder=str(d), status="failed", error=err)
                notify(report(row, {}, "failed", err, "The video is made; run the upload step by hand.", tag)["message"])
                continue
            (d / ".ok-upload").touch()
            up = json.loads((d / "upload.json").read_text()) if (d / "upload.json").exists() else {}
            append_done(root, log, row, folder=str(d), status="uploaded", error="",
                        main_video_id=up.get("main", ""), short_video_id=up.get("short", ""))
            notify(report(row, up, "uploaded", "", "", tag)["message"])
    finally:
        release_lock(root)


# ---- one-off videos requested from the Telegram bot (never touch topics.csv) ----

REQUESTS_DONE = "requests_done.csv"


def start_request(root: Path, today: date, row: dict) -> Path:
    """New folder for a Telegram request. Caller must hold the lock."""
    base = root / "videos" / f"{today.isoformat()}-req-{slugify(row)}"
    d, n = base, 2
    while d.exists():
        d, n = base.with_name(f"{base.name}-{n}"), n + 1
    d.mkdir(parents=True)
    topic = {"row": row, "redo_of": None, "request": True,
             "picked_at": datetime.now().isoformat(timespec="seconds")}
    (d / "topic.json").write_text(json.dumps(topic, indent=2))
    return d


def finish_request(root: Path, d: Path, status: str) -> dict:
    """Log the request to requests_done.csv, release the lock, build the report."""
    row = json.loads((d / "topic.json").read_text())["row"]
    up = json.loads((d / "upload.json").read_text()) if (d / "upload.json").exists() else {}
    err = (d / "error.txt").read_text().strip() if (d / "error.txt").exists() else ""
    if status == "failed" and not err:
        err = "unknown error (see the request log in videos/.requests/)"
    if status == "waiting":
        err = ""
        # no quota: the owner uploads this one by hand from Telegram (the Short follows
        # as a preview video), so it never joins the queue for the next quota reset
        status = "manual"
        (d / QUOTA_WAIT).unlink(missing_ok=True)
    done = read_rows(root / REQUESTS_DONE)
    done.append({**row, "finished_at": datetime.now().isoformat(timespec="seconds"),
                 "folder": str(d), "status": status, "error": err,
                 "main_video_id": up.get("main", ""), "short_video_id": up.get("short", "")})
    write_rows(root / REQUESTS_DONE, DONE_COLS, done)
    release_lock(root)
    hint = ("/retry won't help here: there is no real data for this topic. "
            "Send /new with one of the suggestions." if (d / NO_DATA).exists() else
            "To retry without redoing finished steps, send /retry to the bot.")
    if status == "manual":
        return {"status": status, "title": row["title"] or row["youtube_url"], "main_url": "",
                "short_url": "", "error": "", "message": manual_upload_message(d, row)}
    return report(row, up, status, err, hint, tag=" (Telegram request)")


def manual_upload_message(d: Path, row: dict) -> str:
    """Telegram text for a request made while the quota was used up: everything needed
    to upload the Short by hand (YouTube's upload form: title, description, tags)."""
    import youtube
    try:
        meta = json.loads((d / "metadata.json").read_text()).get("short", {})
    except (OSError, ValueError):
        meta = {}
    body = youtube.build_body(meta, short=True) if meta else {"snippet": {}}
    sn = body["snippet"]
    lines = [f"📥 Made, no YouTube quota left today (Telegram request): "
             f"{row['title'] or row['youtube_url']}",
             "The Short follows as a video: save it and upload it yourself (it is NOT queued "
             "for the next quota reset).", "",
             "Title:", sn.get("title", "-"), "",
             "Description:", sn.get("description", "-")[:2500], "",
             "Tags:", ", ".join(sn.get("tags", [])) or "-"]
    if body.get("localizations"):
        lines += ["", "Translated titles (Studio → Subtitles → Add language):"]
        lines += [f"{k}: {v['title']}" for k, v in body["localizations"].items()]
    lines += ["", f"File: {d / 'out' / 'short.mp4'}"]
    text = "\n".join(lines)
    return text if len(text) <= 4000 else text[:3980] + "\n… (cut)"


def last_failed_request(root: Path) -> Path | None:
    """Folder of the latest request, if it failed and still exists."""
    done = read_rows(root / REQUESTS_DONE)
    if not done or done[-1].get("status") != "failed":
        return None
    d = Path(done[-1].get("folder", ""))
    return d if done[-1].get("folder") and (d / "topic.json").exists() else None


def run_request(root: Path, today: date, row: dict | None = None, resume: Path | None = None,
                steps=None, notify=send_telegram, sleep=time.sleep,
                wait_s: int = REQUEST_WAIT_S) -> int:
    """Exit codes: 0 uploaded, 1 failed step, 2 bad request, 4 lock never freed."""
    if resume is not None:
        row = json.loads((resume / "topic.json").read_text())["row"]
    row = {c: (row or {}).get(c, "").strip() for c in TOPIC_COLS}
    row["look"] = row["look"].lower()
    if problem := row_problem(row):
        notify(f"❌ Telegram request rejected: {problem}")
        return 2
    waited = 0
    while not take_lock(root, "request", row["title"] or row["youtube_url"]):
        if waited >= wait_s:
            notify(f"❌ Telegram request gave up: another video was still being made "
                   f"after {wait_s // 3600} h. Send it again later.")
            return 4
        sleep(LOCK_POLL_S)
        waited += LOCK_POLL_S
    try:
        if resume is not None:
            d = resume
            (d / "error.txt").unlink(missing_ok=True)
        else:
            d = start_request(root, today, row)
    except BaseException:
        release_lock(root)
        raise
    return run_steps(root, d, STEPS if steps is None else steps, notify,
                     lambda status: finish_request(root, d, status))


def runs_today(root: Path, today: date) -> dict[str, int]:
    """Daily + request topics finished today, counted by status (uploaded, failed,
    waiting). A failed topic made no video, so it is not a Short."""
    n: dict[str, int] = {}
    for name in ("topics_done.csv", REQUESTS_DONE):
        try:
            rows = read_rows(root / name)
        except BadFile:
            continue
        for r in rows:
            if r.get("finished_at", "").startswith(today.isoformat()):
                st = r.get("status") or "?"
                n[st] = n.get(st, 0) + 1
    return n


def _print(obj) -> None:
    print(json.dumps(obj))


def views_check(root: Path) -> None:
    """2-day views report (automation/views.py); never fails the run."""
    import views
    try:
        views.check(root, date.today())
    except BaseException as e:  # SystemExit when the stats login is missing
        print(f"views check failed: {e}", file=sys.stderr)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=ROOT)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick")
    sub.add_parser("run", help="all steps for the first topic, then notify (what n8n calls)")
    sub.add_parser("auth")
    sub.add_parser("auth-stats", help="one-time read-only login for channel stats")
    gr = sub.add_parser("growth", help="weekly growth review: stats -> analysis -> topics.csv")
    gr.add_argument("--question", default="")
    st = sub.add_parser("stats", help="channel performance (read-only) as JSON")
    st.add_argument("--days", type=int, default=7)
    st.add_argument("--out", type=Path)
    sub.add_parser("views", help="Telegram: views of the Shorts uploaded two nights ago")
    sub.add_parser("scout", help="weekly trend scout: propose timely topics in Telegram")
    rq = sub.add_parser("request", help="one-off video from the Telegram bot; never touches topics.csv")
    rq.add_argument("--style", choices=STYLES)
    rq.add_argument("--title", default="")
    rq.add_argument("--url", default="")
    rq.add_argument("--comment", default="")
    rq.add_argument("--look", default="", choices=LOOKS)
    rq.add_argument("--resume", type=Path, help="retry a failed request folder")
    for step in ("research", "render", "upload"):
        sub.add_parser(step).add_argument("dir", type=Path)
    f = sub.add_parser("finish")
    f.add_argument("dir", type=Path)
    f.add_argument("--status", choices=["uploaded", "failed"], required=True)
    a = ap.parse_args(argv)
    root = a.root.resolve()

    if a.cmd == "pick":
        code, payload = start(root, date.today())
        _print(payload)
        return code
    if a.cmd == "run":
        code = run_all(root, date.today())
        commit_state(root, f"nightly run {date.today()} (exit {code})")
        views_check(root)
        return 0 if code in (3, 4) else code  # empty list / already running are not failures
    if a.cmd == "research":
        d = a.dir.resolve()
        return run_step("research", d, lambda: research(root, d))
    if a.cmd == "render":
        d = a.dir.resolve()
        return run_step("render", d, lambda: render(root, d))
    if a.cmd == "upload":
        d = a.dir.resolve()
        return run_step("upload", d, lambda: _upload(root, d))
    if a.cmd == "request":
        try:
            if a.resume:
                code = run_request(root, date.today(), resume=a.resume.resolve())
            else:
                code = run_request(root, date.today(), row={
                    "style": a.style or "", "title": a.title, "youtube_url": a.url,
                    "comment": a.comment, "look": a.look})
            commit_state(root, f"Telegram request: {a.title or a.url or a.resume} (exit {code})")
            return code
        except Exception as e:  # steps report themselves; this catches everything else
            send_telegram(f"❌ Telegram request crashed: {type(e).__name__}: {e}")
            return 70
    if a.cmd == "scout":
        import scout
        return scout.run_scout(root, date.today())
    if a.cmd == "auth":
        import youtube
        youtube.auth()
        return 0
    if a.cmd == "auth-stats":
        import stats
        stats.auth_stats()
        return 0
    if a.cmd == "growth":
        import growth
        try:
            return growth.run_growth(root, date.today(), a.question)
        except Exception as e:
            send_telegram(f"❌ Growth review crashed: {type(e).__name__}: {e}")
            return 70
    if a.cmd == "views":
        views_check(root)
        return 0
    if a.cmd == "stats":
        import stats
        return stats.main(root, a.days, a.out)
    if a.cmd == "finish":
        _print(finish(root, a.dir.resolve(), a.status))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
