"""Daily video pipeline steps, called in order by n8n (see docs/automation.md).

pick -> research <dir> -> render <dir> -> upload <dir> -> finish <dir> --status ...
Each step prints one JSON line. On failure a step writes <dir>/error.txt and exits 1.
"""
import argparse
import csv
import json
import os
import subprocess
import re
import shutil
import sys
import time
from datetime import date, datetime
from pathlib import Path
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent.parent
TOPIC_COLS = ["style", "title", "youtube_url", "comment"]
DONE_COLS = TOPIC_COLS + ["finished_at", "folder", "status", "main_video_id",
                          "short_video_id", "error"]
STYLES = ("race", "pillars")
LOCK_STALE_S = 6 * 3600


class StepError(Exception):
    pass


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
        return f"style must be race or pillars (got {row.get('style')!r})"
    if not row.get("title") and not row.get("youtube_url"):
        return "need a title or a youtube_url"
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


def take_lock(root: Path) -> bool:
    lock = _lock(root)
    lock.parent.mkdir(parents=True, exist_ok=True)
    if lock.exists() and time.time() - lock.stat().st_mtime < LOCK_STALE_S:
        return False
    lock.write_text(datetime.now().isoformat())
    return True


def release_lock(root: Path) -> None:
    _lock(root).unlink(missing_ok=True)


def move_to_done(root: Path, row: dict, **fields) -> None:
    """Remove the first row equal to `row` from topics.csv and append it to topics_done.csv."""
    topics = read_rows(root / "topics.csv")
    key = [row.get(c, "") for c in TOPIC_COLS]
    for i, r in enumerate(topics):
        if [r.get(c, "") for c in TOPIC_COLS] == key:
            del topics[i]
            break
    write_rows(root / "topics.csv", TOPIC_COLS, topics)
    done = read_rows(root / "topics_done.csv")
    done.append({**row, "finished_at": datetime.now().isoformat(timespec="seconds"), **fields})
    write_rows(root / "topics_done.csv", DONE_COLS, done)


class Invalid(Exception):
    pass


def pick(root: Path, today: date) -> dict:
    """Start a run on the first topic. Caller must hold the lock."""
    topics = read_rows(root / "topics.csv")
    if not topics:
        raise LookupError("empty")
    row = {c: topics[0].get(c, "") for c in TOPIC_COLS}
    if problem := row_problem(row):
        move_to_done(root, row, status="failed", error=f"pick: {problem}")
        raise Invalid(problem)
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
    return [os.environ.get("CLAUDE_BIN", "claude"), "-p", prompt, "--model", "sonnet",
            "--allowedTools", *tools, "--disallowedTools", *denied,
            "--output-format", "json"]


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


def research(root: Path, d: Path, run=subprocess.run, sleep=time.sleep) -> None:
    cmd = claude_cmd(d, root)
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
        raise StepError("claude finished but wrote no metadata.json")
    topic = json.loads((d / "topic.json").read_text()) if (d / "topic.json").exists() else {}
    notes = d / "research_notes.md"
    if topic.get("row", {}).get("comment") and (
            not notes.exists() or "redo fixes" not in notes.read_text().lower()):
        raise StepError("redo: research_notes.md has no 'Redo fixes' section, "
                        "so the comment was not worked on")
    if not ((d / "data.csv").exists() or (d / "pillars.csv").exists()):
        raise StepError("claude finished but wrote no data.csv or pillars.csv")
    v = run([barrace_bin(root), "validate", str(d)], cwd=root, capture_output=True, text=True)
    if v.returncode != 0:
        raise StepError(f"barrace validate failed\n{_tail(v.stdout + (v.stderr or ''))}")


def render(root: Path, d: Path, run=subprocess.run) -> None:
    bb = barrace_bin(root)
    for cmd in ([bb, "render", str(d), "--format", "both"], [bb, "thumb", str(d)]):
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
    move_to_done(root, row, folder=str(d), status=status, error=err,
                 main_video_id=up.get("main", ""), short_video_id=up.get("short", ""))
    release_lock(root)
    name = row["title"] or row["youtube_url"]
    links = {k: studio(up[k]) for k in ("main", "short") if up.get(k)}
    if status == "uploaded":
        msg = (f"✅ Ready to review: {name}\nMain: {links.get('main', '-')}\n"
               f"Short: {links.get('short', '-')}\nWatch both, then publish in YouTube Studio.")
    else:
        msg = (f"❌ FAILED: {name}\n{err[-1500:]}\n"
               "To retry, put the row back at the top of topics.csv.")
        if links:
            msg += "\nPartly uploaded (private): " + " ".join(links.values())
    return {"status": status, "title": name, "main_url": links.get("main", ""),
            "short_url": links.get("short", ""), "error": err, "message": msg}


def start(root: Path, today: date) -> tuple[int, dict]:
    """Lock + pick. Exit codes: 0 picked, 2 invalid row, 3 empty list, 4 locked."""
    try:
        topics = read_rows(root / "topics.csv")
        read_rows(root / "topics_done.csv")
    except BadFile as e:
        return 2, {"invalid": str(e), "message": f"❌ barrace did nothing today: {e}"}
    if not topics:
        return 3, {"empty": True, "message": "barrace: topics.csv is empty, nothing to make today."}
    if not take_lock(root):
        return 4, {"locked": True}
    try:
        return 0, pick(root, today)
    except Invalid as e:
        release_lock(root)
        return 2, {"invalid": str(e), "message": f"❌ barrace skipped a bad topics.csv row: {e}"}


def _upload(root: Path, d: Path) -> None:
    import youtube
    youtube.upload(d)


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


def run_all(root: Path, today: date, steps=None, notify=send_telegram) -> int:
    code, payload = start(root, today)
    if code:
        if "message" in payload:
            notify(payload["message"])
        return code
    d = Path(payload["dir"])
    for name, fn in (STEPS if steps is None else steps):
        marker = d / f".ok-{name}"
        if marker.exists():  # finished in an earlier, failed run of this folder
            continue
        if run_step(name, d, lambda: fn(root, d)):
            notify(finish(root, d, "failed")["message"])
            return 1
        marker.touch()
    notify(finish(root, d, "uploaded")["message"])
    return 0


def _print(obj) -> None:
    print(json.dumps(obj))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=ROOT)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick")
    sub.add_parser("run", help="all steps for the first topic, then notify (what n8n calls)")
    sub.add_parser("auth")
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
    if a.cmd == "auth":
        import youtube
        youtube.auth()
        return 0
    if a.cmd == "finish":
        _print(finish(root, a.dir.resolve(), a.status))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
