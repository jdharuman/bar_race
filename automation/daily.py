"""Daily video pipeline steps, called in order by n8n (see docs/automation.md).

pick -> research <dir> -> render <dir> -> upload <dir> -> finish <dir> --status ...
Each step prints one JSON line. On failure a step writes <dir>/error.txt and exits 1.
"""
import argparse
import csv
import json
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


def read_rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    with path.open(newline="", encoding="utf-8") as f:
        return [{k: (v or "").strip() for k, v in r.items() if k} for r in csv.DictReader(f)
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


def find_previous(row: dict, done_rows: list[dict], root: Path) -> Path | None:
    vid = video_id(row.get("youtube_url", ""))
    for d in reversed(done_rows):
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
    base = root / "videos" / f"{today.isoformat()}-{slugify(row)}"
    d, n = base, 2
    while d.exists():
        d, n = base.with_name(f"{base.name}-{n}"), n + 1
    prev = find_previous(row, read_rows(root / "topics_done.csv"), root) if row["comment"] else None
    if prev:
        shutil.copytree(prev, d, ignore=shutil.ignore_patterns(
            "out", "upload.json", "error.txt", "topic.json"))
    else:
        d.mkdir(parents=True)
    topic = {"row": row, "redo_of": str(prev) if prev else None,
             "picked_at": datetime.now().isoformat(timespec="seconds")}
    (d / "topic.json").write_text(json.dumps(topic, indent=2))
    return {"dir": str(d), "style": row["style"], "title": row["title"],
            "youtube_url": row["youtube_url"], "redo_of": topic["redo_of"]}


def _print(obj) -> None:
    print(json.dumps(obj))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=ROOT)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick")
    a = ap.parse_args(argv)
    root = a.root.resolve()

    if a.cmd == "pick":
        if not read_rows(root / "topics.csv"):
            _print({"empty": True})
            return 3
        if not take_lock(root):
            _print({"locked": True})
            return 4
        try:
            _print(pick(root, date.today()))
            return 0
        except Invalid as e:
            release_lock(root)
            _print({"invalid": str(e), "message": f"barrace: skipped a bad topics.csv row: {e}"})
            return 2
    return 1


if __name__ == "__main__":
    sys.exit(main())
