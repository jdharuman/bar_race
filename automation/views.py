"""2-day view check: after each nightly run, Telegram gets the views of the Shorts
uploaded two nights earlier, next to earlier batches at the same age.

Read-only (the stats login, automation/stats.py); costs 3 API units. Each check is
kept in videos/growth/views.csv so later batches have something to compare with.
"""
import csv
from datetime import date, datetime, timedelta
from pathlib import Path

AGE_DAYS = 2
COLS = ["batch", "video_id", "title", "style", "privacy", "views", "checked"]
COMPARE = 7  # earlier batches shown


def log_path(root: Path) -> Path:
    return root / "videos" / "growth" / "views.csv"


def local_day(published_at: str) -> date:
    """Batch day of an upload: its local date (the run starts after midnight)."""
    return datetime.fromisoformat(published_at.replace("Z", "+00:00")).astimezone().date()


def fetch(root: Path) -> list[dict]:
    """The channel's latest 50 uploads: id, title, local upload day, privacy, views."""
    import stats
    from googleapiclient.discovery import build
    yt = build("youtube", "v3", credentials=stats._creds(), cache_discovery=False)
    ch = yt.channels().list(part="contentDetails", mine=True).execute()["items"][0]
    uploads = ch["contentDetails"]["relatedPlaylists"]["uploads"]
    items = yt.playlistItems().list(part="contentDetails", playlistId=uploads,
                                    maxResults=50).execute()["items"]
    ids = [i["contentDetails"]["videoId"] for i in items]
    if not ids:
        return []
    kinds = stats.kinds(root)
    out = []
    for v in yt.videos().list(part="snippet,statistics,status", id=",".join(ids)).execute()["items"]:
        kind, style = kinds.get(v["id"], ("?", ""))
        out.append({"video_id": v["id"], "title": v["snippet"]["title"],
                    "day": local_day(v["snippet"]["publishedAt"]), "kind": kind, "style": style,
                    "privacy": v["status"]["privacyStatus"],
                    "views": int(v["statistics"].get("viewCount", 0))})
    return out


def history(root: Path) -> dict[str, int]:
    """Batch day -> total views at the 2-day check, from earlier checks."""
    totals = {}
    try:
        with log_path(root).open(newline="") as f:
            for r in csv.DictReader(f):
                totals[r["batch"]] = totals.get(r["batch"], 0) + int(r["views"] or 0)
    except FileNotFoundError:
        pass
    return totals


def record(root: Path, batch: date, rows: list[dict], now: datetime) -> None:
    path = log_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    new = not path.exists()
    with path.open("a", newline="") as f:
        w = csv.DictWriter(f, COLS, extrasaction="ignore")
        if new:
            w.writeheader()
        for r in rows:
            w.writerow({**r, "batch": batch.isoformat(), "checked": now.isoformat(timespec="minutes")})


def _short_day(d: str) -> str:
    return date.fromisoformat(d).strftime("%-d %b")


def message(batch: date, rows: list[dict], earlier: dict[str, int]) -> str:
    total = sum(r["views"] for r in rows)
    lines = [f"📊 Views after 2 days: Shorts uploaded {batch.strftime('%-d %b')}", ""]
    for r in sorted(rows, key=lambda r: -r["views"]):
        tag = f" ({r['style']})" if r["style"] else ""
        lock = " 🔒 still private" if r["privacy"] != "public" else ""
        lines.append(f"• {r['views']:,} {r['title'].replace(' #Shorts', '')}{tag}{lock}")
    lines += ["", f"Total {total:,} (about {round(total / len(rows)):,} per Short)."]
    prev = sorted((d, v) for d, v in earlier.items() if d < batch.isoformat())[-COMPARE:]
    if prev:
        avg = round(sum(v for _, v in prev) / len(prev))
        trend = "above" if total > avg * 1.1 else "below" if total < avg * 0.9 else "about the same as"
        lines.append(f"That's {trend} the earlier batches (average {avg:,}): "
                     + " · ".join(f"{_short_day(d)} {v:,}" for d, v in prev))
    else:
        lines.append("First check, so no earlier batches to compare with yet.")
    private = sum(r["privacy"] != "public" for r in rows)
    if private:
        lines.append(f"{private} of them still private, so they can't get views yet.")
    return "\n".join(lines)


def check(root: Path, today: date, fetch_fn=fetch, notify=None, now=None) -> int:
    """Report the batch uploaded AGE_DAYS before `today` (once; later calls are no-ops)."""
    import daily
    notify = notify or daily.send_telegram
    batch = today - timedelta(days=AGE_DAYS)
    earlier = history(root)
    if batch.isoformat() in earlier:
        return 0
    rows = [r for r in fetch_fn(root) if r["day"] == batch and r["kind"] != "main"]
    if not rows:
        return 0
    record(root, batch, rows, (now or datetime.now)())
    notify(message(batch, rows, earlier))
    return 0
