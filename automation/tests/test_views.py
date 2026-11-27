from datetime import date, datetime

import views


def _rows():
    return [{"video_id": "a", "title": "Richest People #Shorts", "day": date(2026, 11, 25),
             "kind": "short", "style": "race", "privacy": "public", "views": 870},
            {"video_id": "b", "title": "Guess the Flag: Asia #1 | Can You Get 10/10? #Shorts",
             "day": date(2026, 11, 25), "kind": "short", "style": "quiz", "privacy": "private",
             "views": 0},
            {"video_id": "c", "title": "Other day #Shorts", "day": date(2026, 11, 26),
             "kind": "short", "style": "race", "privacy": "public", "views": 5}]


def test_reports_the_batch_from_two_days_ago_once(tmp_path):
    sent = []
    views.check(tmp_path, date(2026, 11, 27), fetch_fn=lambda root: _rows(), notify=sent.append,
                now=lambda: datetime(2026, 11, 27, 6, 0))
    assert len(sent) == 1
    msg = sent[0]
    assert "uploaded 25 Nov" in msg and "870 Richest People (race)" in msg
    assert "Other day" not in msg and "still private" in msg and "First check" in msg
    views.check(tmp_path, date(2026, 11, 27), fetch_fn=lambda root: _rows(), notify=sent.append)
    assert len(sent) == 1  # already reported
    assert views.history(tmp_path) == {"2026-11-25": 870}


def test_compares_with_earlier_batches(tmp_path):
    sent = []
    views.check(tmp_path, date(2026, 11, 27), fetch_fn=lambda root: _rows(), notify=sent.append)
    views.check(tmp_path, date(2026, 11, 28), fetch_fn=lambda root: _rows(), notify=sent.append)
    assert "below the earlier batches (average 870): 25 Nov 870" in sent[1]


def test_nothing_uploaded_that_day_sends_nothing(tmp_path):
    sent = []
    views.check(tmp_path, date(2026, 12, 9), fetch_fn=lambda root: _rows(), notify=sent.append)
    assert sent == []
