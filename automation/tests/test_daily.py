import csv
import json
import os
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

import daily

DAILY = str(Path(daily.__file__))
TODAY = date(2026, 11, 22)


def write_topics(root, rows):
    daily.write_rows(root / "topics.csv", daily.TOPIC_COLS, rows)


def row(style="race", title="Largest Economies", url="", comment=""):
    return {"style": style, "title": title, "youtube_url": url, "comment": comment}


def cli(root, *args):
    return subprocess.run([sys.executable, DAILY, "--root", str(root), *args],
                          capture_output=True, text=True)


def test_pick_creates_folder_and_topic_json(tmp_path):
    write_topics(tmp_path, [row()])
    out = daily.pick(tmp_path, TODAY)
    d = Path(out["dir"])
    assert d == tmp_path / "videos" / "2026-11-22-largest-economies"
    topic = json.loads((d / "topic.json").read_text())
    assert topic["row"]["title"] == "Largest Economies"
    assert topic["redo_of"] is None
    assert daily.read_rows(tmp_path / "topics.csv") == [row()]


def test_pick_same_day_twice_suffixes(tmp_path):
    (tmp_path / "videos" / "2026-11-22-largest-economies").mkdir(parents=True)
    write_topics(tmp_path, [row()])
    out = daily.pick(tmp_path, TODAY)
    assert out["dir"].endswith("2026-11-22-largest-economies-2")


def test_slug_from_url():
    assert daily.slugify(row(title="", url="https://youtu.be/abc123")) == "yt-abc123"
    assert daily.slugify(row(title="GDP: Top 10 (1960–2025)!")) == "gdp-top-10-1960-2025"


def test_video_id_forms():
    assert daily.video_id("https://www.youtube.com/watch?v=q9EjG56OjN4&t=3") == "q9EjG56OjN4"
    assert daily.video_id("https://youtube.com/shorts/abcDEF") == "abcDEF"
    assert daily.video_id("") is None


def test_pick_empty_exit3(tmp_path):
    write_topics(tmp_path, [])
    r = cli(tmp_path, "pick")
    assert r.returncode == 3
    assert json.loads(r.stdout) == {"empty": True}


def test_pick_locked_exit4(tmp_path):
    write_topics(tmp_path, [row()])
    (tmp_path / "videos").mkdir()
    (tmp_path / "videos" / ".lock").write_text("x")
    r = cli(tmp_path, "pick")
    assert r.returncode == 4
    assert json.loads(r.stdout) == {"locked": True}
    assert not (tmp_path / "videos" / "2026-11-22-largest-economies").exists()


def test_stale_lock_ignored(tmp_path):
    write_topics(tmp_path, [row()])
    lock = tmp_path / "videos" / ".lock"
    lock.parent.mkdir()
    lock.write_text("x")
    old = time.time() - 7 * 3600
    os.utime(lock, (old, old))
    out = daily.pick(tmp_path, TODAY)
    assert Path(out["dir"]).is_dir()


def test_invalid_row_moves_to_done(tmp_path):
    write_topics(tmp_path, [row(style="bars", title="X")])
    r = cli(tmp_path, "pick")
    assert r.returncode == 2
    assert daily.read_rows(tmp_path / "topics.csv") == []
    done = daily.read_rows(tmp_path / "topics_done.csv")
    assert done[0]["status"] == "failed"
    assert "style" in done[0]["error"]
    assert not (tmp_path / "videos" / ".lock").exists()


def test_redo_copies_previous(tmp_path):
    prev = tmp_path / "videos" / "2026-11-20-largest-economies"
    (prev / "out").mkdir(parents=True)
    (prev / "data.csv").write_text("year,name,value\n")
    (prev / "out" / "main.mp4").write_text("v")
    (prev / "upload.json").write_text("{}")
    done = {**row(), "finished_at": "x", "folder": str(prev), "status": "uploaded",
            "main_video_id": "m", "short_video_id": "s", "error": ""}
    daily.write_rows(tmp_path / "topics_done.csv", daily.DONE_COLS, [done])
    write_topics(tmp_path, [row(title="largest  economies", comment="India missing")])
    out = daily.pick(tmp_path, TODAY)
    d = Path(out["dir"])
    assert (d / "data.csv").exists()
    assert not (d / "out").exists()
    assert not (d / "upload.json").exists()
    assert out["redo_of"] == str(prev)
