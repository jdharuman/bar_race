"""One-off videos requested from the Telegram bot: never touch topics.csv,
share the run lock with the daily video, and never cost the daily video."""
import json
import time
from datetime import date

import daily

TODAY = date(2026, 11, 22)


def row(style="race", title="Most Visited Countries", url="", comment=""):
    return {"style": style, "title": title, "youtube_url": url, "comment": comment}


def uploaded(root, d):
    (d / "upload.json").write_text(json.dumps({"main": "m1", "short": "s1"}))


def ok(root, d):
    pass


STEPS = [("research", ok), ("render", ok), ("upload", uploaded)]


def test_request_success_leaves_topics_csv_alone(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [row(title="Queued A")])
    before = (tmp_path / "topics.csv").read_bytes()
    sent = []
    assert daily.run_request(tmp_path, TODAY, row=row(), steps=STEPS, notify=sent.append) == 0
    assert (tmp_path / "topics.csv").read_bytes() == before
    assert not (tmp_path / "topics_done.csv").exists()
    done = daily.read_rows(tmp_path / daily.REQUESTS_DONE)
    assert done[0]["status"] == "uploaded" and done[0]["main_video_id"] == "m1"
    assert "✅" in sent[0] and "Telegram request" in sent[0]
    d = tmp_path / "videos" / "2026-11-22-req-most-visited-countries"
    assert json.loads((d / "topic.json").read_text())["request"] is True
    assert not (tmp_path / "videos" / ".lock").exists()


def test_request_without_topics_csv(tmp_path):
    assert daily.run_request(tmp_path, TODAY, row=row(), steps=STEPS, notify=[].append) == 0
    assert not (tmp_path / "topics.csv").exists()


def test_bad_request_rejected(tmp_path):
    sent = []
    assert daily.run_request(tmp_path, TODAY, row=row(style="pie"), steps=STEPS,
                             notify=sent.append) == 2
    assert "rejected" in sent[0]
    assert not (tmp_path / "videos" / ".lock").exists()


def test_failed_request_then_retry_resumes_folder(tmp_path):
    calls = []

    def research(root, d):
        calls.append("research")

    def bad_upload(root, d):
        raise daily.StepError("quotaExceeded")
    sent = []
    assert daily.run_request(tmp_path, TODAY, row=row(), notify=sent.append,
                             steps=[("research", research), ("upload", bad_upload)]) == 1
    assert "FAILED" in sent[0] and "/retry" in sent[0]
    d = daily.last_failed_request(tmp_path)
    assert d is not None and (d / "error.txt").exists()

    assert daily.run_request(tmp_path, TODAY, resume=d, notify=sent.append,
                             steps=[("research", research), ("upload", uploaded)]) == 0
    assert calls == ["research"]  # research not redone
    assert not (d / "error.txt").exists()
    assert daily.last_failed_request(tmp_path) is None
    assert len(list((tmp_path / "videos").glob("*-req-*"))) == 1


def test_request_waits_for_running_video(tmp_path):
    assert daily.take_lock(tmp_path, "daily", "GDP")
    slept = []

    def sleep(s):
        slept.append(s)
        if len(slept) == 2:
            daily.release_lock(tmp_path)  # the daily video finishes
    assert daily.run_request(tmp_path, TODAY, row=row(), steps=STEPS, notify=[].append,
                             sleep=sleep) == 0
    assert len(slept) == 2


def test_request_gives_up_after_wait(tmp_path):
    assert daily.take_lock(tmp_path, "daily", "GDP")
    sent = []
    assert daily.run_request(tmp_path, TODAY, row=row(), steps=STEPS, notify=sent.append,
                             sleep=lambda s: None, wait_s=120) == 4
    assert "gave up" in sent[0]
    assert daily.lock_holder(tmp_path)["kind"] == "daily"  # not stolen


def test_daily_waits_for_request_instead_of_skipping(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [row(title="GDP")])
    assert daily.take_lock(tmp_path, "request", "Most Visited")
    slept = []

    def sleep(s):
        slept.append(s)
        daily.release_lock(tmp_path)  # the request finishes
    sent = []
    assert daily.run_all(tmp_path, TODAY, steps=STEPS, notify=sent.append, sleep=sleep) == 0
    assert slept == [daily.LOCK_POLL_S]
    assert daily.read_rows(tmp_path / "topics_done.csv")[0]["title"] == "GDP"
    assert "✅" in sent[0]


def test_daily_still_skips_when_another_daily_holds_lock(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [row(title="GDP")])
    assert daily.take_lock(tmp_path, "daily", "GDP")
    slept = []
    assert daily.run_all(tmp_path, TODAY, steps=STEPS, notify=[].append,
                         sleep=slept.append) == 4
    assert slept == []


def test_daily_gives_up_on_long_request_and_says_so(tmp_path):
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [row(title="GDP")])
    assert daily.take_lock(tmp_path, "request", "Most Visited")
    sent = []
    assert daily.run_all(tmp_path, TODAY, steps=STEPS, notify=sent.append,
                         sleep=lambda s: None, wait_s=120) == 4
    assert "skipped today's video" in sent[0]
    assert daily.read_rows(tmp_path / "topics.csv")[0]["title"] == "GDP"  # still queued


def test_lock_holder_and_stale(tmp_path):
    assert daily.lock_holder(tmp_path) is None
    daily.take_lock(tmp_path, "request", "X")
    assert daily.lock_holder(tmp_path)["title"] == "X"
    old = time.time() - daily.LOCK_STALE_S - 10
    lock = tmp_path / "videos" / ".lock"
    import os
    os.utime(lock, (old, old))
    assert daily.lock_holder(tmp_path) is None


def test_runs_today_counts_both_logs(tmp_path):
    daily.run_request(tmp_path, date.today(), row=row(), steps=STEPS, notify=[].append)
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, [row(title="GDP")])
    daily.run_all(tmp_path, date.today(), steps=STEPS, notify=[].append)
    assert daily.runs_today(tmp_path, date.today()) == {"uploaded": 2}
