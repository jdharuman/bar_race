"""Weekly themes, three nightly slots and the YouTube quota guard."""
import json
from datetime import date, datetime, timezone
from pathlib import Path

import daily
import youtube

MON = date(2026, 11, 23)  # sports
TUE = date(2026, 11, 24)  # india


def row(style="race", title="T", theme="", look=""):
    return {"style": style, "title": title, "youtube_url": "", "comment": "", "look": look, "theme": theme}


def topics(root, rows):
    daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, rows)


def uploaded(root, d):
    (d / "upload.json").write_text(json.dumps({"main": "m-" + d.name, "short": "s-" + d.name}))


def ok(root, d):
    pass


STEPS = [("research", ok), ("render", ok), ("upload", uploaded)]


def test_theme_of_week():
    assert daily.theme_of(MON) == "sports" and daily.theme_of(TUE) == "india"
    assert daily.theme_of(date(2026, 11, 29)) == "people"  # Sunday


def test_choose_prefers_theme_then_unthemed_then_any():
    t = [row("race", "A", "india"), row("race", "B", ""), row("race", "C", "sports"), row("pillars", "P", "sports")]
    assert daily.choose(t, ("race",), "sports") == 2
    assert daily.choose(t, ("race",), "earth") == 1
    assert daily.choose([row("race", "A", "india")], ("race",), "earth") == 0
    assert daily.choose(t, ("stocks", "compare"), "sports") is None
    assert daily.choose(t) == 0


def test_night_makes_one_pair_per_slot_for_todays_theme(tmp_path):
    topics(tmp_path, [row("race", "R india", "india"), row("race", "R sports", "sports"),
                      row("pillars", "P sports", "sports"), row("stocks", "A vs B", "sports"),
                      row("compare", "C vs D", "india")])
    sent = []
    assert daily.run_all(tmp_path, MON, steps=STEPS, notify=sent.append) == 0
    done = [r["title"] for r in daily.read_rows(tmp_path / "topics_done.csv")]
    assert done == ["R sports", "P sports", "A vs B"]
    left = [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")]
    assert left == ["R india", "C vs D"]
    assert sum("✅" in m for m in sent) == 3
    assert all("Playlist: Sports" in m for m in sent if "✅" in m)


def test_missing_slot_reported_once(tmp_path):
    topics(tmp_path, [row("race", "R", "sports")])
    sent = []
    assert daily.run_all(tmp_path, MON, steps=STEPS, notify=sent.append) == 0
    notes = [m for m in sent if "no pillars / lines topic" in m]
    assert len(notes) == 1 and "Sports" in notes[0]


def test_bad_rows_moved_out_with_notice(tmp_path):
    topics(tmp_path, [row("pie", "Bad"), row("race", "Good"), row("race", "Neon race", look="neon")])
    sent = []
    daily.run_all(tmp_path, MON, steps=STEPS, notify=sent.append, slots=["race"])
    failed = [r["title"] for r in daily.read_rows(tmp_path / "topics_done.csv") if r["status"] == "failed"]
    assert failed == ["Bad", "Neon race"]
    assert sum("bad topics.csv row" in m for m in sent) == 2
    assert any("Good" in m and "✅" in m for m in sent)


def test_bad_theme_rejected():
    assert "theme must be" in daily.row_problem(row(theme="cooking"))
    assert daily.row_problem(row(theme="Sports")) is None  # case-blind


def test_capitalised_style_and_theme_work(tmp_path):
    topics(tmp_path, [row("Race", "R", "Sports")])
    sent = []
    assert daily.run_all(tmp_path, MON, steps=STEPS, notify=sent.append, slots=["race"]) == 0
    assert daily.read_rows(tmp_path / "topics_done.csv")[0]["status"] == "uploaded"
    assert daily.read_rows(tmp_path / "topics.csv") == []


# ---- quota ----

class FakeYT:
    def __init__(self):
        self.calls = []

    def insert(self, path, body):
        self.calls.append("insert")
        return f"v{len(self.calls)}"

    def set_thumbnail(self, vid, path):
        self.calls.append("thumb")


def folder(tmp_path, name="v"):
    d = tmp_path / "videos" / name
    (d / "out").mkdir(parents=True)
    (d / "metadata.json").write_text(json.dumps({"main": {"title": "M"}, "short": {"title": "S"}}))
    return d


def test_upload_spends_and_stops_before_overdraw(tmp_path):
    q = daily.Quota(tmp_path, limit=4000, day="2026-11-22")
    d = folder(tmp_path)
    yt = FakeYT()
    youtube.upload_all(d, yt, q)
    assert yt.calls == ["insert", "insert", "thumb"] and q.used == 3250
    d2 = folder(tmp_path, "w")
    try:
        youtube.upload_all(d2, FakeYT(), q)
        raise AssertionError("should wait")
    except daily.StepError as e:
        assert "waiting for YouTube quota" in str(e)
    assert (d2 / daily.QUOTA_WAIT).exists()
    # ledger persisted and resets on a new Pacific day
    assert daily.Quota(tmp_path, limit=4000, day="2026-11-22").used == 3250
    assert daily.Quota(tmp_path, limit=4000, day="2026-11-23").used == 0


def test_quota_day_is_pacific():
    # 02:00 Berlin on Nov 23 is still Nov 22 in California
    t = datetime(2026, 11, 23, 1, 0, tzinfo=timezone.utc)
    assert daily.quota_day(t) == "2026-11-22"


def test_held_upload_then_uploaded_next_run(tmp_path):
    topics(tmp_path, [row("race", "R", "sports"), row("pillars", "P", "sports")])
    budget = {"left": 1}  # first night: only the first pair fits

    def upload(root, d):
        if budget["left"] <= 0:
            (d / daily.QUOTA_WAIT).write_text("x")
            raise daily.StepError("waiting for YouTube quota")
        budget["left"] -= 1
        uploaded(root, d)
    steps = [("research", ok), ("render", ok), ("upload", upload)]
    sent = []
    assert daily.run_all(tmp_path, MON, steps=steps, notify=sent.append) == 0
    status = {r["title"]: r["status"] for r in daily.read_rows(tmp_path / "topics_done.csv")}
    assert status == {"R": "uploaded", "P": "waiting"}
    assert any("⏳" in m and "P" in m for m in sent)
    assert not any(p.name == "error.txt" for p in (tmp_path / "videos").rglob("*"))

    budget["left"] = 5  # quota reset; topics.csv is empty of slot topics now
    topics(tmp_path, [row("race", "R2", "india")])
    sent.clear()
    daily.run_all(tmp_path, TUE, steps=steps, notify=sent.append)
    rows = daily.read_rows(tmp_path / "topics_done.csv")
    latest = {}
    for r in rows:
        latest[r["title"]] = r["status"]
    assert latest == {"R": "uploaded", "P": "uploaded", "R2": "uploaded"}
    assert sent[0].startswith("✅") and "P" in sent[0]  # held video goes first


def test_request_held_for_quota_is_not_a_failure(tmp_path):
    def upload(root, d):
        (d / daily.QUOTA_WAIT).write_text("x")
        raise daily.StepError("waiting for YouTube quota")
    sent = []
    code = daily.run_request(tmp_path, MON, row=row("race", "Req"), notify=sent.append,
                             steps=[("research", ok), ("upload", upload)])
    assert code == 0 and "⏳" in sent[0] and "Telegram request" in sent[0]
    assert daily.read_rows(tmp_path / daily.REQUESTS_DONE)[0]["status"] == "waiting"
    assert daily.last_failed_request(tmp_path) is None


def test_unsourceable_topic_makes_way_for_the_next_one_once(tmp_path):
    topics(tmp_path, [row("race", "R1", "sports"), row("race", "R2", "sports"),
                      row("race", "R3", "sports")])

    def no_data(root, d):  # research gave up on every topic
        (d / daily.NO_DATA).touch()
        raise daily.StepError("research gave up")
    sent = []
    code = daily.run_all(tmp_path, MON, steps=[("research", no_data), ("render", ok),
                                               ("upload", uploaded)],
                         notify=sent.append, slots=["race"])
    assert code == 1
    done = daily.read_rows(tmp_path / "topics_done.csv")
    assert [(r["title"], r["status"]) for r in done] == [("R1", "failed"), ("R2", "failed")]
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == ["R3"]
    assert sum("trying the next race topic" in m for m in sent) == 1


def test_other_failures_do_not_try_another_topic(tmp_path):
    topics(tmp_path, [row("race", "R1", "sports"), row("race", "R2", "sports")])

    def broken(root, d):
        raise daily.StepError("render exited 1")
    assert daily.run_all(tmp_path, MON, steps=[("research", ok), ("render", broken)],
                         notify=lambda m: None, slots=["race"]) == 1
    assert [r["title"] for r in daily.read_rows(tmp_path / "topics.csv")] == ["R2"]
