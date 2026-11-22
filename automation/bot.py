"""Telegram bot: make a one-off video on demand, independent of topics.csv.

/new asks style -> title -> YouTube link (optional) -> notes (optional), then
"Make now" starts `daily.py request ...` in the background. The daily 02:00
video from topics.csv is unaffected (it waits if a request is still running).

Only the owner (TELEGRAM_CHAT_ID) is obeyed; everyone else is ignored.
Text typed into the bot is data for the research step, never instructions
(CLAUDE.md rule 6). Runs as the `barrace-bot` systemd user service.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import date, datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import daily  # noqa: E402

ROOT = Path(os.environ.get("BARRACE_REPO") or Path(__file__).resolve().parent.parent)
MAX_TITLE, MAX_COMMENT = 100, 500
STYLE_BUTTONS = [("Race chart", "style:race"), ("Pillars", "style:pillars"),
                 ("Stocks", "style:stocks"), ("Compare", "style:compare")]


def companies(text: str) -> list[str]:
    """'Apple vs Microsoft, Nvidia' -> ['Apple', 'Microsoft', 'Nvidia']."""
    parts = re.split(r"\s+vs\.?\s+|\s*,\s*|\s+and\s+|\s+&\s+", text, flags=re.I)
    return [p.strip() for p in parts if p.strip()]
REPORTED = (0, 1, 2, 4, 70)  # exit codes after which `daily.py request` already sent a message

HELP = ("barrace bot\n"
        "/new – make a one-off video now (not from topics.csv)\n"
        "/status – is a video being made right now?\n"
        "/retry – retry the last failed request (finished steps are kept)\n"
        "/cancel – stop filling in /new\n"        "Mondays: trend ideas arrive here; ✅ adds one to the top of topics.csv.\n"
        "The daily 02:00 video from topics.csv keeps running as usual.")
COMMANDS = [("new", "Make a one-off video"), ("status", "Is a video being made?"),
            ("retry", "Retry the last failed request"), ("cancel", "Stop filling in /new"),
            ("help", "What this bot does")]


class Telegram:
    def __init__(self, token: str):
        self.base = f"https://api.telegram.org/bot{token}/"

    def call(self, method: str, **params):
        req = urllib.request.Request(self.base + method, json.dumps(params).encode(),
                                     {"Content-Type": "application/json"})
        # long polls hold the connection for `timeout` s; allow a margin on top
        with urllib.request.urlopen(req, timeout=params.get("timeout", 0) + 30) as r:
            body = json.loads(r.read())
        if not body.get("ok"):
            raise RuntimeError(f"{method}: {body.get('description')}")
        return body["result"]


def buttons(*rows):
    return {"inline_keyboard": [[{"text": t, "callback_data": d} for t, d in row] for row in rows]}


SKIP = buttons([("Skip", "skip")])


def summary(r: dict) -> str:
    look = (f"\nLook: {'black neon' if r.get('look') == 'neon' else 'light'}"
            if r["style"] in daily.LINE_STYLES else "")
    return (f"Style: {r['style']}\nTitle: {r['title']}{look}\n"
            f"YouTube hint: {r['youtube_url'] or '–'}\nNotes: {r['comment'] or '–'}")


def launch(root: Path, args: list[str], log_dir: Path) -> subprocess.Popen:
    """Start `daily.py request ...` detached, so a bot restart doesn't kill the video."""
    log_dir.mkdir(parents=True, exist_ok=True)
    log = (log_dir / f"{datetime.now():%Y-%m-%dT%H-%M-%S}.log").open("w")
    return subprocess.Popen([sys.executable, str(root / "automation" / "daily.py"),
                             "--root", str(root), "request", *args],
                            cwd=root, stdin=subprocess.DEVNULL, stdout=log,
                            stderr=subprocess.STDOUT, start_new_session=True)


class Bot:
    def __init__(self, tg, owner: int, root: Path = ROOT, start=launch, today=date.today):
        self.tg, self.owner, self.root, self.start, self.today = tg, owner, root, start, today
        self.forms: dict[int, dict] = {}      # chat id -> {"step", "row"}
        self.children: list[subprocess.Popen] = []

    # -- plumbing --
    def say(self, chat: int, text: str, markup=None):
        params = {"chat_id": chat, "text": text}
        if markup:
            params["reply_markup"] = markup
        self.tg.call("sendMessage", **params)

    def handle(self, update: dict) -> None:
        if cq := update.get("callback_query"):
            chat = (cq.get("message") or {}).get("chat", {}).get("id")
            if cq.get("from", {}).get("id") != self.owner or chat != self.owner:
                return
            self.tg.call("answerCallbackQuery", callback_query_id=cq["id"])
            self.on_button(chat, cq.get("data", ""))
            return
        msg = update.get("message") or {}
        chat = msg.get("chat", {}).get("id")
        if msg.get("from", {}).get("id") != self.owner or chat != self.owner:
            return  # not the owner: ignore silently
        text = (msg.get("text") or "").strip()
        if text.startswith("/"):
            self.on_command(chat, text.split()[0].split("@")[0].lower())
        elif chat in self.forms:
            self.on_text(chat, text)
        else:
            self.say(chat, HELP)

    # -- commands --
    def on_command(self, chat: int, cmd: str) -> None:
        if cmd == "/new":
            self.forms[chat] = {"step": "style", "row": {c: "" for c in daily.TOPIC_COLS}}
            self.say(chat, "New one-off video (not added to topics.csv).\nWhich style?",
                     buttons(STYLE_BUTTONS, [("Cancel", "cancel")]))
        elif cmd == "/cancel":
            self.say(chat, "Cancelled." if self.forms.pop(chat, None) else "Nothing to cancel.")
        elif cmd == "/status":
            self.say(chat, self.status())
        elif cmd == "/retry":
            d = daily.last_failed_request(self.root)
            if d is None:
                self.say(chat, "No failed request to retry.")
                return
            self.run(chat, [f"--resume={d}"], f"Retrying: {d.name}")
        else:
            self.say(chat, HELP)

    def status(self) -> str:
        h = daily.lock_holder(self.root)
        busy = (f"Making a video now: {h.get('title') or '?'} "
                f"({'Telegram request' if h.get('kind') == 'request' else 'daily video'}, "
                f"since {h.get('since', '?')[11:16]})") if h else "Idle: no video being made."
        n = daily.runs_today(self.root, self.today())
        q = daily.Quota(self.root)
        return (f"{busy}\nFinished today: {n}\n"
                f"YouTube quota today: {q.used:,} of {q.limit:,} units used "
                f"(a video pair needs about {2 * daily.COST_VIDEO + daily.COST_THUMB:,}).")

    # -- form --
    def on_button(self, chat: int, data: str) -> None:
        if data.startswith("scout:"):  # weekly trend scout idea: add / skip
            import scout
            self.say(chat, scout.decide(self.root, data))
            return
        form = self.forms.get(chat)
        if data == "cancel":
            self.forms.pop(chat, None)
            self.say(chat, "Cancelled.")
            return
        if form is None:
            self.say(chat, "That form has expired. Send /new to start again.")
            return
        step, r = form["step"], form["row"]
        if step == "style" and data in [d for _, d in STYLE_BUTTONS]:
            r["style"] = data.split(":")[1]
            form["step"] = "title"
            if r["style"] == "stocks":
                self.say(chat, "Which companies? 2 or 3, e.g. Apple vs Microsoft vs Nvidia\n"
                               "(shows what $1,000 invested in each would be worth, since all were listed)")
            elif r["style"] == "compare":
                self.say(chat, "What to compare over time? 2–3 things and the measure, e.g.\n"
                               "Messi vs Ronaldo career goals, or Tamil Nadu vs Karnataka GDP")
            else:
                self.say(chat, f"Title? (up to {MAX_TITLE} characters)\n"
                               "e.g. Most Visited Countries in the World")
        elif step == "look" and data in ("look:neon", "look:light"):
            r["look"] = data.split(":")[1]
            self.ask_url(chat, form)
        elif step == "url" and data == "skip":
            self.ask_comment(chat, form)
        elif step == "comment" and data == "skip":
            self.confirm(chat, form)
        elif step == "confirm" and data == "go":
            self.forms.pop(chat, None)
            # --opt=value: a value starting with "-" must not look like an option
            self.run(chat, [f"--style={r['style']}", f"--title={r['title']}",
                            f"--url={r['youtube_url']}", f"--comment={r['comment']}",
                            f"--look={r['look']}"],
                     f"Starting: {r['title']}")
        else:
            self.say(chat, "Please answer the current question, or /cancel.")

    def on_text(self, chat: int, text: str) -> None:
        form = self.forms[chat]
        step, r = form["step"], form["row"]
        text = " ".join(text.split())  # one line: newlines would break topics/CSV fields
        if step == "title":
            if not text or len(text) > MAX_TITLE:
                self.say(chat, f"The title must be 1–{MAX_TITLE} characters. Try again.")
                return
            if r["style"] == "stocks":
                names = companies(text)
                if not 2 <= len(names) <= 3:
                    self.say(chat, "Give 2 or 3 companies, e.g. Apple vs Microsoft vs Nvidia.")
                    return
                text = " vs ".join(names)
            r["title"] = text
            if r["style"] in daily.LINE_STYLES:
                form["step"] = "look"
                self.say(chat, "Look?", buttons([("⬛ Black neon", "look:neon"), ("⬜ Light", "look:light")]))
            else:
                self.ask_url(chat, form)
        elif step == "url":
            if not daily.video_id(text):
                self.say(chat, "That isn't a YouTube video link. Send one, or tap Skip.", SKIP)
                return
            r["youtube_url"] = text
            self.ask_comment(chat, form)
        elif step == "comment":
            if len(text) > MAX_COMMENT:
                self.say(chat, f"Notes must be at most {MAX_COMMENT} characters.", SKIP)
                return
            r["comment"] = text
            self.confirm(chat, form)
        else:
            self.say(chat, "Please use the buttons above, or /cancel.")

    def ask_url(self, chat: int, form: dict) -> None:
        form["step"] = "url"
        self.say(chat, "YouTube link as a topic hint? (optional; only the topic is used, "
                       "never the video's numbers or frames)", SKIP)

    def ask_comment(self, chat: int, form: dict) -> None:
        form["step"] = "comment"
        self.say(chat, "Notes for the research? (optional, e.g. 'use UN Tourism data, "
                       "2000–2024')", SKIP)

    def confirm(self, chat: int, form: dict) -> None:
        form["step"] = "confirm"
        if problem := daily.row_problem(form["row"]):
            self.forms.pop(chat, None)
            self.say(chat, f"Can't make that: {problem}. Send /new to start again.")
            return
        self.say(chat, summary(form["row"]) + "\n\nMake this video now?",
                 buttons([("▶ Make now", "go"), ("✖ Cancel", "cancel")]))

    def run(self, chat: int, args: list[str], what: str) -> None:
        h = daily.lock_holder(self.root)
        wait = (f"\nAnother video is being made ({h.get('title') or '?'}); yours starts "
                "automatically when it finishes.") if h else ""
        n = daily.runs_today(self.root, self.today())
        quota = ("\n⚠️ Already 2+ videos today: YouTube's daily upload quota "
                 "(about 3) may run out." if n >= 2 else "")
        child = self.start(self.root, args, self.root / "videos" / ".requests")
        self.children.append(child)
        self.say(chat, f"{what}{wait}{quota}\nIt takes about 10–60 min. You'll get the "
                       "YouTube Studio links here (uploads stay private).")

    def reap(self) -> None:
        """Report a request process that died without sending its own report."""
        alive = []
        for child in self.children:
            code = child.poll()
            if code is None:
                alive.append(child)
            elif code not in REPORTED:
                self.say(self.owner, f"❌ The request process crashed (exit {code}). "
                                     "See videos/.requests/ for its log.")
        self.children = alive


def load_offset(path: Path) -> int | None:
    try:
        return int(path.read_text())
    except (FileNotFoundError, ValueError):
        return None


def main() -> int:
    token, owner = os.environ.get("TELEGRAM_BOT_TOKEN"), os.environ.get("TELEGRAM_CHAT_ID")
    if not token or not owner or not owner.lstrip("-").isdigit():
        print("TELEGRAM_BOT_TOKEN and a numeric TELEGRAM_CHAT_ID are required", file=sys.stderr)
        return 2
    tg = Telegram(token)
    bot = Bot(tg, int(owner))
    state = ROOT / "videos" / ".bot_offset"
    state.parent.mkdir(parents=True, exist_ok=True)
    offset = load_offset(state)
    try:
        tg.call("setMyCommands", commands=[{"command": c, "description": d} for c, d in COMMANDS])
        if offset is None:  # first start: drop old backlog
            old = tg.call("getUpdates", offset=-1, timeout=0)
            offset = old[-1]["update_id"] + 1 if old else 0
    except Exception as e:
        print(f"startup: {e}", file=sys.stderr)
    print("barrace bot running", flush=True)
    while True:
        try:
            updates = tg.call("getUpdates", timeout=40, offset=offset or 0,
                              allowed_updates=["message", "callback_query"])
        except (urllib.error.URLError, TimeoutError, OSError, RuntimeError) as e:
            print(f"getUpdates: {e}", file=sys.stderr, flush=True)
            time.sleep(10)
            updates = []
        for u in updates:
            offset = u["update_id"] + 1
            state.write_text(str(offset))
            try:
                bot.handle(u)
            except Exception as e:  # one bad update must not stop the bot
                print(f"update {u.get('update_id')}: {type(e).__name__}: {e}",
                      file=sys.stderr, flush=True)
        bot.reap()


if __name__ == "__main__":
    sys.exit(main())
