"""Weekly trend scout: Claude proposes up to 5 timely topics; the owner approves each
in Telegram (✅ puts it at the top of topics.csv, so it runs on its theme's next day).

The agent only reads the web and writes one JSON file. Its proposals are data:
each is checked like any topics.csv row (CLAUDE.md rule 6) before it is shown.
"""
import json
import os
import shutil
import subprocess
import time
from datetime import date
from pathlib import Path

import daily

MAX_IDEAS = 5
SCOUT_TIMEOUT_S = 15 * 60


def scout_dir(root: Path) -> Path:
    return root / "videos" / "scout"  # not a dot-folder: permission patterns skip those


def claude_cmd(out: Path) -> list[str]:
    prompt = (
        "Read .claude/skills/barrace-scout/SKILL.md and follow it exactly. "
        f"Write your proposals as JSON to {out} and nothing else."
    )
    # a per-run folder: Claude Code matches folder patterns, not single-file rules
    # file-write permission is granted through Edit rules too (as in daily.claude_cmd)
    tools = ["Read", "Glob", "Grep", "WebSearch", "WebFetch",
             f"Write(/{out.parent}/**)", f"Edit(/{out.parent}/**)"]
    denied = ["Read(~/.config/barrace/**)", "Read(~/.ssh/**)", "Bash"]
    return [os.environ.get("CLAUDE_BIN", "claude"), "-p", prompt, "--model", "sonnet",
            "--setting-sources", "project", "--allowedTools", *tools,
            "--disallowedTools", *denied, "--output-format", "json"]


def known_titles(root: Path) -> set[str]:
    titles = set()
    for name in ("topics.csv", "topics_done.csv", daily.REQUESTS_DONE):
        try:
            titles |= {daily._norm(r.get("title", "")) for r in daily.read_rows(root / name)}
        except daily.BadFile:
            continue
    return titles


def check(raw, known: set[str]) -> list[dict]:
    """Keep proposals that are valid rows with a theme and are not already planned."""
    items = raw.get("ideas", raw) if isinstance(raw, dict) else raw
    good, seen = [], set(known)
    for it in items if isinstance(items, list) else []:
        if not isinstance(it, dict):
            continue
        row = {c: str(it.get(c, "")).strip() for c in daily.TOPIC_COLS}
        row["style"], row["look"], row["theme"] = row["style"].lower(), row["look"].lower(), row["theme"].lower()
        row["comment"] = ""  # a comment means "redo" to the research step
        row["title"] = " ".join(row["title"].split())[:80]
        if (row["style"] == "quiz" or not row["theme"] or daily.row_problem(row)
                or daily._norm(row["title"]) in seen):
            continue
        seen.add(daily._norm(row["title"]))
        good.append({**row, "why": " ".join(str(it.get("why", "")).split())[:200],
                     "source": " ".join(str(it.get("source", "")).split())[:120], "status": "pending"})
        if len(good) == MAX_IDEAS:
            break
    return good


def idea_text(i: int, n: int, it: dict) -> str:
    return (f"💡 Trend idea {i + 1}/{n} – {daily.THEMES[it['theme']]}, {it['style']}\n"
            f"{it['title']}\nWhy now: {it['why'] or '–'}\nData: {it['source'] or '–'}\n"
            "✅ adds it to the top of topics.csv (made on the next "
            f"{daily.THEMES[it['theme']]} day).")


def run_scout(root: Path, today: date, run=subprocess.run, send=None, notify=daily.send_telegram) -> int:
    out_dir = scout_dir(root)
    out_dir.mkdir(parents=True, exist_ok=True)
    run_dir = out_dir / f"{today.isoformat()}-run"
    shutil.rmtree(run_dir, ignore_errors=True)
    run_dir.mkdir()
    raw_path = run_dir / "ideas.json"
    env = {k: os.environ[k] for k in daily.AGENT_ENV if k in os.environ}
    before = daily.snapshot(root)
    try:
        r = run(claude_cmd(raw_path), cwd=root, env=env, capture_output=True, text=True,
                timeout=SCOUT_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        notify("❌ Trend scout timed out; no ideas this week.")
        return 1
    (out_dir / f"{today.isoformat()}.log").write_text(f"{r.stdout or ''}\n{r.stderr or ''}")
    if daily.snapshot(root) != before:
        notify("❌ Trend scout changed tool files; ideas discarded. Check `git diff`.")
        return 1
    try:
        raw = json.loads(raw_path.read_text())
    except (FileNotFoundError, ValueError):
        notify(f"❌ Trend scout wrote no usable proposals (claude exited {r.returncode}).")
        return 1
    ideas = check(raw, known_titles(root))
    path = out_dir / f"{today.isoformat()}.json"
    path.write_text(json.dumps({"date": today.isoformat(), "ideas": ideas}, indent=2))
    if not ideas:
        notify("Trend scout: nothing new and usable this week.")
        return 0
    send = send or send_with_buttons
    for i, it in enumerate(ideas):
        send(idea_text(i, len(ideas), it), [[("✅ Add", f"scout:{today.isoformat()}:{i}:y"),
                                             ("❌ Skip", f"scout:{today.isoformat()}:{i}:n")]])
        time.sleep(0.2)
    return 0


def send_with_buttons(text: str, rows) -> None:
    import bot
    token, chat = os.environ.get("TELEGRAM_BOT_TOKEN"), os.environ.get("TELEGRAM_CHAT_ID")
    print(text)
    if token and chat:
        try:
            bot.Telegram(token).call("sendMessage", chat_id=int(chat), text=text,
                                     reply_markup=bot.buttons(*rows))
        except Exception as e:  # a lost message must not fail the scout
            print(f"telegram send failed: {e}")


def decide(root: Path, data: str) -> str:
    """Handle a button `scout:<date>:<i>:<y|n>`; returns the reply text."""
    try:
        _, day, i, yes = data.split(":")
        i = int(i)
        date.fromisoformat(day)  # no path tricks via the date part
    except ValueError:
        return "That button is broken."
    path = scout_dir(root) / f"{day}.json"
    with daily.topics_lock(root):
        try:
            st = json.loads(path.read_text())
            it = st["ideas"][i]
        except (FileNotFoundError, ValueError, IndexError, KeyError):
            return "That idea has expired."
        if it.get("status") != "pending":
            return f"Already {it.get('status')}: {it['title']}"
        if yes == "y":
            topics = daily.read_rows(root / "topics.csv")
            row = {c: it.get(c, "") for c in daily.TOPIC_COLS}
            daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, [row, *topics])
            it["status"] = "added"
            reply = f"✅ Added to the top of topics.csv: {it['title']} ({daily.THEMES[it['theme']]} day)"
        else:
            it["status"] = "skipped"
            reply = f"Skipped: {it['title']}"
        path.write_text(json.dumps(st, indent=2))
    return reply
