"""Weekly growth review: channel stats -> Claude analysis -> topics.csv adjusted.

`daily.py growth` (n8n, Sundays) or `/grow [question]` in the bot:
1. pull 28 days of channel stats (read-only login, automation/stats.py) and
   precompute the tables, so the agent never does arithmetic;
2. a headless Claude run (.claude/skills/barrace-growth) reads them, checks
   current trends on the web and writes one JSON plan;
3. the plan is data (CLAUDE.md rule 6): only these actions are applied, each
   checked here: promote existing topics (run sooner), add new topics (checked
   like scout ideas), drop weak topics (kept in videos/growth/dropped.csv);
4. Telegram gets the report and an Undo button.

It never touches YouTube (titles, privacy, publishing stay with the owner).
"""
import csv
import json
import os
import shutil
import subprocess
from datetime import date, datetime
from pathlib import Path

import daily
import scout

DAYS = 28
MAX_PROMOTE, MAX_ADD, MAX_DROP, MAX_TIPS = 10, 5, 10, 6
TIMEOUT_S = 20 * 60
TG_LIMIT = 3900


def growth_dir(root: Path) -> Path:
    return root / "videos" / "growth"


def claude_cmd(run_dir: Path) -> list[str]:
    prompt = ("Read .claude/skills/barrace-growth/SKILL.md and follow it exactly. "
              f"Your input files are in {run_dir}. Write your plan as JSON to "
              f"{run_dir / 'plan.json'} and nothing else.")
    tools = ["Read", "Glob", "Grep", "WebSearch", "WebFetch",
             f"Write(/{run_dir}/**)", f"Edit(/{run_dir}/**)"]
    denied = ["Read(~/.config/barrace/**)", "Read(~/.ssh/**)", "Bash"]
    return [os.environ.get("CLAUDE_BIN", "claude"), "-p", prompt, "--model", "sonnet",
            "--setting-sources", "project", "--allowedTools", *tools,
            "--disallowedTools", *denied, "--output-format", "json"]


def inputs(root: Path, run_dir: Path, data: dict, question: str) -> None:
    """Write what the agent reads: precomputed stats, the topic queue, the question."""
    import stats
    rows = stats.video_table(data)
    topics = daily.read_rows(root / "topics.csv")
    counts = {}
    for r in topics:
        counts[r.get("style", "").lower()] = counts.get(r.get("style", "").lower(), 0) + 1
    (run_dir / "stats.json").write_text(json.dumps({
        "channel": data["channel"], "range": data["range"],
        "analytics_note": data.get("analytics_error") or "Analytics lags 1-2 days.",
        "nightly_slots": daily.graph_slots() + ["quiz (fills the remaining quota)"],
        "by_style": stats.by_style(rows), "videos": rows, "per_day": data["per_day"],
        "traffic": data["traffic"], "audience": data.get("audience", {}),
        "topics_left": counts,
        "next_topics": [{k: r.get(k, "") for k in ("style", "title", "theme")}
                        for r in topics[:60]],
    }, indent=1))
    (run_dir / "question.txt").write_text(question.strip() or "(none: do the weekly review)")


def _find(topics: list[dict], title: str) -> int | None:
    key = daily._norm(title)
    return next((i for i, r in enumerate(topics) if daily._norm(r.get("title", "")) == key), None)


def _text(x, n: int) -> str:
    return " ".join(str(x or "").split())[:n]


def check(plan, topics: list[dict], known: set[str]) -> dict:
    """Keep only well-formed actions on real topics; everything else is dropped."""
    plan = plan if isinstance(plan, dict) else {}
    out = {"report": str(plan.get("report", "")).strip()[:3000],
           "tips": [_text(t, 300) for t in plan.get("tips", []) if _text(t, 300)][:MAX_TIPS],
           "promote": [], "drop": [], "add": []}
    used = set()
    for name, cap in (("promote", MAX_PROMOTE), ("drop", MAX_DROP)):
        for it in plan.get(name, []) if isinstance(plan.get(name), list) else []:
            title = _text(it.get("title") if isinstance(it, dict) else it, 120)
            i = _find(topics, title)
            if i is None or daily._norm(title) in used or len(out[name]) == cap:
                continue
            used.add(daily._norm(title))
            out[name].append({"title": topics[i]["title"],
                              "why": _text(it.get("why") if isinstance(it, dict) else "", 160)})
    out["add"] = [{k: v for k, v in r.items() if k != "status"}
                  for r in scout.check({"ideas": plan.get("add", [])}, known)][:MAX_ADD]
    return out


def apply(root: Path, run_dir: Path, plan: dict) -> None:
    """Change topics.csv under its lock; keep a copy for Undo."""
    with daily.topics_lock(root):
        topics = daily.read_rows(root / "topics.csv")
        shutil.copy(root / "topics.csv", run_dir / "topics.before.csv")
        dropped = []
        for d in plan["drop"]:
            if (i := _find(topics, d["title"])) is not None:
                dropped.append(topics.pop(i))
        moved = [topics.pop(i) for p in reversed(plan["promote"])
                 if (i := _find(topics, p["title"])) is not None][::-1]
        added = [{c: a.get(c, "") for c in daily.TOPIC_COLS} for a in plan["add"]]
        daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, [*added, *moved, *topics])
        if dropped:
            log = growth_dir(root) / "dropped.csv"
            new = not log.exists()
            with log.open("a", newline="") as f:
                w = csv.DictWriter(f, [*daily.TOPIC_COLS, "dropped_on"], extrasaction="ignore")
                if new:
                    w.writeheader()
                for r in dropped:
                    w.writerow({**r, "dropped_on": run_dir.name[:10]})
        state = {"added": [a["title"] for a in added], "dropped": dropped,
                 "promoted": [m["title"] for m in moved], "undone": False}
        (run_dir / "applied.json").write_text(json.dumps(state, indent=1))


def undo(root: Path, data: str) -> str:
    """Button `growth:<run>:undo`: remove the added topics, put dropped ones back.
    (Promoted topics just keep their place; their order is harmless.)"""
    try:
        _, run, _ = data.split(":")
        if len(run) != 19:
            raise ValueError(run)
        datetime.strptime(run, "%Y-%m-%dT%H-%M-%S")  # no path tricks
    except ValueError:
        return "That button is broken."
    path = growth_dir(root) / run / "applied.json"
    with daily.topics_lock(root):
        try:
            st = json.loads(path.read_text())
        except (FileNotFoundError, ValueError):
            return "Nothing to undo for that review."
        if st.get("undone"):
            return "Already undone."
        topics = daily.read_rows(root / "topics.csv")
        added = {daily._norm(t) for t in st["added"]}
        topics = [r for r in topics if daily._norm(r.get("title", "")) not in added]
        back = [{c: r.get(c, "") for c in daily.TOPIC_COLS} for r in st["dropped"]]
        daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, [*topics, *back])
        st["undone"] = True
        path.write_text(json.dumps(st, indent=1))
    daily.commit_state(root, f"undo growth review {run}")
    return (f"↩️ Undone: removed {len(added)} added topic(s), put {len(back)} dropped "
            "topic(s) back at the end of topics.csv.")


def message(plan: dict) -> str:
    lines = ["📈 Weekly growth review", "", plan["report"] or "(no report)"]
    if plan["tips"]:
        lines += ["", "💡 To get more subscribers:"] + [f"• {t}" for t in plan["tips"]]
    ch = []
    ch += [f"⬆️ {p['title']}" + (f" – {p['why']}" if p["why"] else "") for p in plan["promote"]]
    ch += [f"➕ {a['title']} ({a['style']}, {a['theme']})" + (f" – {a['why']}" if a.get("why") else "")
           for a in plan["add"]]
    ch += [f"➖ {d['title']}" + (f" – {d['why']}" if d["why"] else "") for d in plan["drop"]]
    lines += ["", "🛠 topics.csv changes (already applied):"] + (ch or ["none"])
    text = "\n".join(lines)
    return text if len(text) <= TG_LIMIT else text[:TG_LIMIT - 20] + "\n… (cut)"


def run_growth(root: Path, today: date, question: str = "", run=subprocess.run,
               collect=None, send=None, notify=daily.send_telegram, now=None) -> int:
    import stats
    stamp = (now or datetime.now)().strftime("%Y-%m-%dT%H-%M-%S")
    run_dir = growth_dir(root) / stamp
    run_dir.mkdir(parents=True)
    try:
        data = (collect or stats.collect)(root, DAYS)
    except BaseException as e:  # SystemExit when the stats login is missing
        notify(f"❌ Growth review: could not read channel stats: {e}")
        return 1
    inputs(root, run_dir, data, question)
    env = {k: os.environ[k] for k in daily.AGENT_ENV if k in os.environ}
    before = daily.snapshot(root)
    try:
        r = run(claude_cmd(run_dir), cwd=root, env=env, capture_output=True, text=True,
                timeout=TIMEOUT_S)
    except subprocess.TimeoutExpired:
        notify("❌ Growth review timed out; nothing changed.")
        return 1
    (run_dir / "claude.log").write_text(f"{r.stdout or ''}\n{r.stderr or ''}")
    if daily.snapshot(root) != before:
        notify("❌ Growth review changed tool files; plan discarded. Check `git diff`.")
        return 1
    try:
        raw = json.loads((run_dir / "plan.json").read_text())
    except (FileNotFoundError, ValueError):
        notify(f"❌ Growth review wrote no usable plan (claude exited {r.returncode}).")
        return 1
    plan = check(raw, daily.read_rows(root / "topics.csv"), scout.known_titles(root))
    (run_dir / "checked.json").write_text(json.dumps(plan, indent=1))
    apply(root, run_dir, plan)
    daily.commit_state(root, f"growth review {stamp}: +{len(plan['add'])} added, "
                             f"{len(plan['promote'])} promoted, -{len(plan['drop'])} dropped")
    changed = plan["promote"] or plan["add"] or plan["drop"]
    rows = [[("↩️ Undo topic changes", f"growth:{stamp}:undo")]] if changed else []
    (send or scout.send_with_buttons)(message(plan), rows)
    return 0
