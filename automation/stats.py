"""Read-only channel performance: views, watch time and retention per video.

Uses its own login (youtube_stats_token.json, read-only scopes) so the upload
login stays upload-only. Never changes anything on YouTube.
"""
import csv
import json
from datetime import date, timedelta
from pathlib import Path

from youtube import CONFIG

SCOPES = ["https://www.googleapis.com/auth/youtube.readonly",
          "https://www.googleapis.com/auth/yt-analytics.readonly"]
TOKEN = CONFIG / "youtube_stats_token.json"
REAUTH = "No stats login: run `uv run --project automation automation/daily.py auth-stats`"
METRICS = ("views", "estimatedMinutesWatched", "averageViewDuration", "averageViewPercentage",
           "likes", "comments", "shares", "subscribersGained")


def auth_stats() -> None:
    """One-time read-only login (same flow and port as `daily.py auth`)."""
    from google_auth_oauthlib.flow import InstalledAppFlow
    secret = CONFIG / "client_secret.json"
    if not secret.exists():
        raise SystemExit(f"missing {secret}: download the OAuth desktop client JSON there")
    creds = InstalledAppFlow.from_client_secrets_file(str(secret), SCOPES).run_local_server(
        port=8080, open_browser=False)
    TOKEN.write_text(creds.to_json())
    TOKEN.chmod(0o600)
    print(f"saved {TOKEN}")


def _creds():
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials
    if not TOKEN.exists():
        raise SystemExit(REAUTH)
    creds = Credentials.from_authorized_user_file(str(TOKEN), SCOPES)
    if not creds.valid:
        creds.refresh(Request())
        TOKEN.write_text(creds.to_json())
    return creds


def kinds(root: Path) -> dict[str, tuple[str, str]]:
    """video id -> (kind, style) from the local logs: kind is short/main."""
    out = {}
    for name in ("topics_done.csv", "requests_done.csv"):
        try:
            rows = list(csv.DictReader((root / name).open(newline="")))
        except FileNotFoundError:
            continue
        for r in rows:
            for k in ("short", "main"):
                if vid := r.get(f"{k}_video_id"):
                    out[vid] = (k, (r.get("style") or "").lower())
    return out


def collect(root: Path, days: int, today: date | None = None) -> dict:
    from googleapiclient.discovery import build
    creds = _creds()
    yt = build("youtube", "v3", credentials=creds, cache_discovery=False)
    ya = build("youtubeAnalytics", "v2", credentials=creds, cache_discovery=False)
    today = today or date.today()
    start, end = (today - timedelta(days=days)).isoformat(), today.isoformat()

    ch = yt.channels().list(part="snippet,statistics,contentDetails", mine=True).execute()["items"][0]
    uploads = ch["contentDetails"]["relatedPlaylists"]["uploads"]
    ids, token = [], None
    while len(ids) < 200:
        page = yt.playlistItems().list(part="contentDetails", playlistId=uploads, maxResults=50,
                                       pageToken=token).execute()
        ids += [i["contentDetails"]["videoId"] for i in page["items"]]
        token = page.get("nextPageToken")
        if not token:
            break
    videos = {}
    for i in range(0, len(ids), 50):
        for v in yt.videos().list(part="snippet,statistics,status,contentDetails",
                                  id=",".join(ids[i:i + 50])).execute()["items"]:
            videos[v["id"]] = {"title": v["snippet"]["title"],
                               "published": v["snippet"]["publishedAt"][:10],
                               "privacy": v["status"]["privacyStatus"],
                               "duration": v["contentDetails"]["duration"],
                               "views_total": int(v["statistics"].get("viewCount", 0)),
                               "likes_total": int(v["statistics"].get("likeCount", 0))}

    def query(**kw):
        r = ya.reports().query(ids="channel==MINE", startDate=start, endDate=end, **kw).execute()
        cols = [c["name"] for c in r.get("columnHeaders", [])]
        return [dict(zip(cols, row)) for row in r.get("rows", [])]

    from googleapiclient.errors import HttpError
    try:  # Analytics needs its own API enabled in the Cloud project; totals work without it
        per_video = query(metrics=",".join(METRICS), dimensions="video", sort="-views",
                          maxResults=200)
        per_day = query(metrics="views,estimatedMinutesWatched,subscribersGained,subscribersLost",
                        dimensions="day", sort="day")
        traffic = query(metrics="views", dimensions="insightTrafficSourceType", sort="-views")
        audience = {
            "countries": query(metrics="views,averageViewPercentage,subscribersGained",
                               dimensions="country", sort="-views", maxResults=25),
            "age_gender": query(metrics="viewerPercentage", dimensions="ageGroup,gender"),
            "subscribed": query(metrics="views,averageViewPercentage",
                                dimensions="subscribedStatus"),
        }
        analytics_error = ""
    except HttpError as e:
        per_video = per_day = traffic = []
        audience = {}
        analytics_error = (e.reason or str(e))[:300]
    return {"channel": {"title": ch["snippet"]["title"], **ch["statistics"]},
            "range": [start, end], "videos": videos, "per_video": per_video,
            "per_day": per_day, "traffic": traffic, "audience": audience,
            "kinds": kinds(root), "analytics_error": analytics_error}


def video_table(data: dict) -> list[dict]:
    """One row per upload in the range: our style/kind, lifetime totals and the
    range's Analytics numbers (computed here, so the agent never does the sums)."""
    an = {r["video"]: r for r in data["per_video"]}
    rows = []
    for vid, v in data["videos"].items():
        if v["published"] < data["range"][0]:
            continue
        kind, style = data["kinds"].get(vid, ("?", "?"))
        a = an.get(vid, {})
        rows.append({"id": vid, "title": v["title"], "published": v["published"],
                     "privacy": v["privacy"], "kind": kind, "style": style,
                     "views_total": v["views_total"], "likes_total": v["likes_total"],
                     "views": a.get("views", 0), "avg_pct_viewed": a.get("averageViewPercentage"),
                     "avg_seconds": a.get("averageViewDuration"),
                     "subs_gained": a.get("subscribersGained", 0), "shares": a.get("shares", 0)})
    return sorted(rows, key=lambda r: r["published"], reverse=True)


def by_style(rows: list[dict]) -> dict:
    """Averages per style for public Shorts (views_total and avg % viewed)."""
    out = {}
    for r in rows:
        if r["kind"] != "short" or r["privacy"] != "public":
            continue
        g = out.setdefault(r["style"], {"n": 0, "views_total": 0, "pct": [], "subs": 0})
        g["n"] += 1
        g["views_total"] += r["views_total"]
        g["subs"] += r["subs_gained"]
        if r["avg_pct_viewed"] is not None:
            g["pct"].append(r["avg_pct_viewed"])
    return {k: {"shorts": g["n"], "avg_views": round(g["views_total"] / g["n"]),
                "avg_pct_viewed": round(sum(g["pct"]) / len(g["pct"]), 1) if g["pct"] else None,
                "subs_gained": g["subs"]} for k, g in out.items()}


def main(root: Path, days: int, out: Path | None) -> int:
    data = collect(root, days)
    text = json.dumps(data, indent=1)
    if out:
        out.write_text(text)
        print(f"saved {out}")
    else:
        print(text)
    return 0
