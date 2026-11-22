"""Flag / capital quiz Shorts: generator, pipeline routing, Short-only upload, quota gate."""
import collections
import csv
import json
import os
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

import daily
import quiz
import youtube

ROOT = Path(__file__).resolve().parents[2]


def test_parse_title():
    assert quiz.parse_title("Guess the Flag: Europe #3") == ("flag", "europe", 3)
    assert quiz.parse_title("guess the  capital :  Indian   States  # 12") == ("capital", "indian states", 12)
    for bad in ("Guess the Flag: Mars #1", "Guess the Flag: Europe", "Flags of Europe #2",
                "Guess the Capital: Look-Alikes #1", "Guess the Flag: Europe #0"):
        with pytest.raises(quiz.QuizError):
            quiz.parse_title(bad)


def test_every_planned_quiz_title_builds():
    rows = daily.read_rows(ROOT / "topics.plan.csv")
    titles = [r["title"] for r in rows if r["style"] == "quiz"]
    assert len(titles) >= 365
    for t in titles:
        _, _, qs = quiz.build(t)
        assert len(qs) == quiz.QUESTIONS
        assert len({q["key"] for q in qs}) == quiz.QUESTIONS, t
        for q in qs:
            assert len(set(q["choices"])) == 4 and q["choices"][q["correct"] - 1] == q["answer"]


def test_build_is_deterministic_and_rotates_through_the_set():
    assert quiz.build("Guess the Flag: Asia #4") == quiz.build("Guess the Flag: Asia #4")
    seen = collections.Counter()
    n_pool = len(quiz.pool("flag", "europe"))
    for n in range(1, n_pool // quiz.QUESTIONS + 1):  # no repeats until the set is used up
        for q in quiz.build(f"Guess the Flag: Europe #{n}")[2]:
            seen[q["key"]] += 1
    assert max(seen.values()) == 1


def test_sets_hold_what_their_names_say():
    regions = {q["region"] for q in quiz.pool("flag", "africa")}
    assert regions == {"africa"}
    assert all(q["group"] for q in quiz.pool("flag", "look-alikes"))
    caps = {q["answer"] for q in quiz.pool("capital", "world")}
    assert "Paris" in caps and not any("/" in c for c in caps)  # multi-capital countries left out
    states = quiz.pool("capital", "indian states")
    assert len(states) == 28 and not any(q["image"] for q in states)


def test_lookalike_questions_use_lookalike_wrong_answers():
    _, _, qs = quiz.build("Guess the Flag: Look-Alikes #1")
    pool = {q["answer"]: q["group"] for q in quiz.pool("flag", "look-alikes")}
    for q in qs:
        same = [c for c in q["choices"] if c != q["answer"] and pool.get(c) == q["group"]]
        group_size = sum(1 for g in pool.values() if g == q["group"])
        assert len(same) == min(3, group_size - 1), q


def fake_fetch(iso2, dest, cache):
    dest.write_bytes(b"\x89PNG fake " + iso2.encode())


def test_generate_writes_a_complete_short_folder(tmp_path):
    d = tmp_path / "v"
    d.mkdir()
    quiz.generate(tmp_path, d, "Guess the Flag: Europe #3", fetch=fake_fetch)
    rows = list(csv.DictReader((d / "quiz.csv").open()))
    assert len(rows) == 10 and all((d / r["image"]).exists() for r in rows)
    assert all(r["fact"].startswith("Capital") for r in rows)
    toml = (d / "video.toml").read_text()
    assert 'style = "quiz"' in toml and 'title = "Guess the Flag"' in toml and "reviewed = true" in toml
    meta = json.loads((d / "metadata.json").read_text())
    assert set(meta) == {"short"}  # Short only
    assert "public domain" in meta["short"]["description"] and "CC0" in meta["short"]["description"]
    assert "Wikidata" in (d / "research_notes.md").read_text()


def test_generated_folder_passes_barrace_validate(tmp_path):
    bb = ROOT / "target" / "release" / "barrace"
    if not bb.exists():
        pytest.skip("barrace not built")
    d = tmp_path / "v"
    d.mkdir()
    quiz.generate(tmp_path, d, "Guess the Capital: Indian States #2", fetch=fake_fetch)
    r = subprocess.run([str(bb), "validate", str(d)], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr
    assert "questions: 10" in r.stdout


def quiz_dir(tmp_path, title="Guess the Flag: Europe #1"):
    d = tmp_path / "videos" / "q"
    d.mkdir(parents=True)
    (d / "topic.json").write_text(json.dumps({"row": {"style": "quiz", "title": title}}))
    return d


def test_research_step_generates_without_claude(tmp_path, monkeypatch):
    d = quiz_dir(tmp_path)
    monkeypatch.setattr(quiz, "fetch_flag", fake_fetch)
    calls = []

    def run(cmd, **kw):
        calls.append(cmd)
        return SimpleNamespace(returncode=0, stdout="ok", stderr="")
    daily.research(tmp_path, d, run=run, sleep=lambda s: None)
    assert [c[1] for c in calls] == ["validate"]  # no claude
    assert (d / "quiz.csv").exists()


def test_research_step_reports_a_bad_quiz_title(tmp_path):
    d = quiz_dir(tmp_path, "Guess the Flag: Atlantis #1")
    with pytest.raises(daily.StepError, match="unknown flag quiz"):
        daily.research(tmp_path, d, run=None, sleep=None)


def test_render_step_makes_the_short_only(tmp_path):
    d = quiz_dir(tmp_path)
    calls = []

    def run(cmd, **kw):
        calls.append(cmd)
        return SimpleNamespace(returncode=0, stdout="", stderr="")
    daily.render(tmp_path, d, run=run)
    assert [c[1:] for c in calls] == [["render", str(d), "--format", "short"]]


class FakeYT:
    def __init__(self):
        self.inserted, self.thumbs = [], []

    def insert(self, path, body):
        self.inserted.append(path.name)
        return "id-" + path.stem

    def set_thumbnail(self, vid, path):
        self.thumbs.append(vid)


def test_upload_short_only_no_thumbnail(tmp_path):
    (tmp_path / "out").mkdir()
    (tmp_path / "out" / "short.mp4").write_text("x")
    (tmp_path / "metadata.json").write_text(json.dumps({"short": {"title": "Guess the Flag: Asia #1"}}))
    yt = FakeYT()
    q = daily.Quota(tmp_path, limit=10000, day="2026-11-23")
    state = youtube.upload_all(tmp_path, yt, q)
    assert yt.inserted == ["short.mp4"] and yt.thumbs == []
    assert state == {"short": "id-short"} and q.used == daily.COST_VIDEO


def test_quiz_report_names_the_short_only():
    msg = daily.report({"style": "quiz", "title": "Guess the Flag: Asia #1", "youtube_url": ""},
                       {"short": "abc"}, "uploaded", "", "")["message"]
    assert "Short: https://studio.youtube.com/video/abc/edit" in msg and "Main" not in msg


def test_quiz_slot_waits_for_quota(tmp_path, monkeypatch):
    monkeypatch.delenv("BARRACE_QUIZ", raising=False)
    monkeypatch.setenv("BARRACE_QUOTA", "10000")
    assert not daily.quiz_slot_on(tmp_path)
    monkeypatch.setenv("BARRACE_QUOTA", str(daily.QUIZ_MIN_QUOTA))
    assert daily.quiz_slot_on(tmp_path)
    monkeypatch.setenv("BARRACE_QUIZ", "off")
    assert not daily.quiz_slot_on(tmp_path)
    monkeypatch.setenv("BARRACE_QUOTA", "10000")
    monkeypatch.setenv("BARRACE_QUIZ", "on")
    assert daily.quiz_slot_on(tmp_path)


def _night(tmp_path, monkeypatch, quota):
    monkeypatch.delenv("BARRACE_QUIZ", raising=False)
    monkeypatch.setenv("BARRACE_QUOTA", str(quota))
    rows = [{"style": "race", "title": "R", "theme": "sports"},
            {"style": "quiz", "title": "Guess the Flag: Europe #1"}]
    daily.write_rows(tmp_path / "topics.csv", daily.TOPIC_COLS, rows)

    def up(root, d):
        (d / "upload.json").write_text(json.dumps({"short": "s"}))
    steps = [("research", lambda r, d: None), ("render", lambda r, d: None), ("upload", up)]
    sent = []
    from datetime import date
    assert daily.run_all(tmp_path, date(2026, 11, 23), steps=steps, notify=sent.append) == 0
    return [r["title"] for r in daily.read_rows(tmp_path / "topics_done.csv")], sent


def test_night_adds_the_quiz_only_with_enough_quota(tmp_path, monkeypatch):
    done, sent = _night(tmp_path, monkeypatch, 10000)
    assert done == ["R"]
    assert not any("quiz" in m for m in sent)  # a switched-off slot is not "missing"
    (tmp_path / "more").mkdir()
    done, _ = _night(tmp_path / "more", monkeypatch, 20000)
    assert done == ["R", "Guess the Flag: Europe #1"]
