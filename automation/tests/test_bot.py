from datetime import date

import bot
import daily

OWNER = 111
STRANGER = 999


class FakeTG:
    def __init__(self):
        self.calls = []

    def call(self, method, **params):
        self.calls.append((method, params))
        return True

    def texts(self):
        return [p["text"] for m, p in self.calls if m == "sendMessage"]

    def last_buttons(self):
        for m, p in reversed(self.calls):
            if m == "sendMessage" and "reply_markup" in p:
                return [b["callback_data"] for row in p["reply_markup"]["inline_keyboard"]
                        for b in row]
        return []


class FakeChild:
    def __init__(self, code=None):
        self.code = code

    def poll(self):
        return self.code


def make(tmp_path):
    tg, started = FakeTG(), []

    def start(root, args, log_dir):
        started.append(args)
        return FakeChild()
    return bot.Bot(tg, OWNER, root=tmp_path, start=start, today=lambda: date(2026, 11, 22)), tg, started


def msg(text, who=OWNER):
    return {"message": {"from": {"id": who}, "chat": {"id": who}, "text": text}}


def tap(data, who=OWNER):
    return {"callback_query": {"id": "q", "from": {"id": who}, "data": data,
                               "message": {"chat": {"id": who}}}}


def fill(b, *updates):
    for u in updates:
        b.handle(u)


def test_full_form_starts_request_with_exact_args(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:race"), msg("Most Visited Countries"),
         tap("skip"), msg("use UN Tourism data\n2000-2024"))
    assert tg.last_buttons() == ["go", "cancel"]
    assert "Most Visited Countries" in tg.texts()[-1]
    b.handle(tap("go"))
    assert started == [["--style=race", "--title=Most Visited Countries", "--url=",
                        "--comment=use UN Tourism data 2000-2024", "--look="]]
    assert "Starting" in tg.texts()[-1]
    assert not (tmp_path / "topics.csv").exists()  # never queued


def test_youtube_hint_validated(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:pillars"), msg("Tallest Dams"), msg("not a link"))
    assert "isn't a YouTube video link" in tg.texts()[-1]
    fill(b, msg("https://youtu.be/abc123"), tap("skip"), tap("go"))
    assert started[0][2] == "--url=https://youtu.be/abc123"
    assert started[0][0] == "--style=pillars"


def test_stranger_ignored(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new", STRANGER), tap("style:race", STRANGER), tap("go", STRANGER))
    assert tg.calls == [] and started == []


def test_owner_in_other_chat_ignored(tmp_path):
    b, tg, _ = make(tmp_path)
    b.handle({"message": {"from": {"id": OWNER}, "chat": {"id": -100}, "text": "/new"}})
    assert tg.calls == []


def test_title_length_enforced(tmp_path):
    b, tg, _ = make(tmp_path)
    fill(b, msg("/new"), tap("style:race"), msg("x" * 101))
    assert "1–100" in tg.texts()[-1]
    assert b.forms[OWNER]["step"] == "title"


def test_cancel_and_stale_buttons(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:race"), msg("/cancel"), tap("go"))
    assert "expired" in tg.texts()[-1] and started == []


def test_go_only_valid_at_confirm(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("go"))
    assert started == [] and "current question" in tg.texts()[-1]


def test_status_idle_and_busy(tmp_path):
    b, tg, _ = make(tmp_path)
    b.handle(msg("/status"))
    assert "Idle" in tg.texts()[-1]
    daily.take_lock(tmp_path, "daily", "GDP")
    b.handle(msg("/status"))
    assert "GDP" in tg.texts()[-1] and "daily video" in tg.texts()[-1]


def test_busy_warns_request_will_wait(tmp_path):
    b, tg, started = make(tmp_path)
    daily.take_lock(tmp_path, "daily", "GDP")
    fill(b, msg("/new"), tap("style:race"), msg("X"), tap("skip"), tap("skip"), tap("go"))
    assert started and "starts automatically" in tg.texts()[-1]


def test_retry_uses_last_failed_folder(tmp_path):
    b, tg, started = make(tmp_path)
    b.handle(msg("/retry"))
    assert "No failed request" in tg.texts()[-1]

    def bad(root, d):
        raise daily.StepError("boom")
    daily.run_request(tmp_path, date(2026, 11, 22), notify=[].append, steps=[("research", bad)],
                      row={"style": "race", "title": "X", "youtube_url": "", "comment": ""})
    b.handle(msg("/retry"))
    assert started[-1][0].startswith("--resume=") and started[-1][0].endswith("-req-x")


def test_reap_reports_only_unreported_crashes(tmp_path):
    b, tg, _ = make(tmp_path)
    b.children = [FakeChild(0), FakeChild(1), FakeChild(70), FakeChild(-9), FakeChild(None)]
    b.reap()
    assert len(b.children) == 1
    assert sum("crashed" in t for t in tg.texts()) == 1


def test_unknown_text_gets_help(tmp_path):
    b, tg, _ = make(tmp_path)
    b.handle(msg("hello"))
    assert "/new" in tg.texts()[-1]


def test_dash_title_reaches_daily_cli_intact(tmp_path):
    """Real argv round trip through daily.py's parser."""
    import subprocess, sys
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:race"), msg("-40 C Coldest Cities"), tap("skip"),
         msg("--help is not an option here"), tap("go"))
    r = subprocess.run([sys.executable, daily.__file__, "--root", str(tmp_path), "request",
                        *started[0][:1], "--style=pie", *started[0][1:]],
                       capture_output=True, text=True)
    assert r.returncode == 2 and "invalid choice" in r.stderr  # only --style=pie rejected
    parsed = daily.argparse.ArgumentParser()
    parsed.add_argument("--title"); parsed.add_argument("--comment")
    ns, _ = parsed.parse_known_args(started[0])
    assert ns.title == "-40 C Coldest Cities" and ns.comment == "--help is not an option here"


def test_stocks_form_normalises_companies(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"))
    assert "style:stocks" in tg.last_buttons()
    fill(b, tap("style:stocks"))
    assert "Which companies" in tg.texts()[-1]
    fill(b, msg("Apple"))
    assert "2 or 3" in tg.texts()[-1]
    fill(b, msg("apple, Microsoft and NVIDIA"))
    assert tg.last_buttons() == ["look:neon", "look:light"]
    fill(b, tap("look:neon"), tap("skip"), tap("skip"))
    assert "black neon" in tg.texts()[-1]
    fill(b, tap("go"))
    assert started[0][:2] == ["--style=stocks", "--title=apple vs Microsoft vs NVIDIA"]
    assert started[0][-1] == "--look=neon"


def test_race_form_never_asks_look(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:race"), msg("GDP"))
    assert "YouTube link" in tg.texts()[-1]
    fill(b, tap("look:neon"))  # stale/foreign button: ignored
    assert "current question" in tg.texts()[-1]


def test_companies_split():
    assert bot.companies("Apple vs. Microsoft & Tesla") == ["Apple", "Microsoft", "Tesla"]
    assert bot.companies("Coca-Cola VS Pepsi") == ["Coca-Cola", "Pepsi"]
    assert bot.companies("AT&T vs Verizon") == ["AT&T", "Verizon"]


def test_compare_form_asks_look(tmp_path):
    b, tg, started = make(tmp_path)
    fill(b, msg("/new"), tap("style:compare"))
    assert "What to compare" in tg.texts()[-1]
    fill(b, msg("Messi vs Ronaldo career goals"))
    assert tg.last_buttons() == ["look:neon", "look:light"]
    fill(b, tap("look:light"), tap("skip"), tap("skip"), tap("go"))
    assert started[0][0] == "--style=compare" and started[0][-1] == "--look=light"


def test_status_shows_quota(tmp_path):
    b, tg, _ = make(tmp_path)
    daily.Quota(tmp_path).spend(3250)
    b.handle(msg("/status"))
    assert "3,250 of 10,000" in tg.texts()[-1]
