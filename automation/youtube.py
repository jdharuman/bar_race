"""YouTube upload: always private. Never publishes, never deletes."""
import json
import os
from pathlib import Path

from daily import COST_THUMB, COST_VIDEO, QUOTA_WAIT, Quota, StepError

CONFIG = Path(os.environ.get("BARRACE_CONFIG", Path.home() / ".config" / "barrace"))
SCOPES = ["https://www.googleapis.com/auth/youtube.upload"]
REAUTH = "YouTube login expired or missing: run `uv run --project automation automation/daily.py auth`"


def build_body(meta: dict, short: bool) -> dict:
    title, desc = meta.get("title", "").strip(), meta.get("description", "").strip()
    if short:
        if "#shorts" not in title.lower() and len(title) <= 92:
            title += " #Shorts"
        if "#shorts" not in desc.lower():
            desc = f"{desc}\n\n#Shorts".strip()
    tags, used = [], 0
    for t in meta.get("tags", []):
        if used + len(t) + 1 > 500:
            break
        tags.append(t)
        used += len(t) + 1
    return {"snippet": {"title": title[:100], "description": desc[:5000], "tags": tags,
                        "categoryId": "27"},
            "status": {"privacyStatus": "private", "selfDeclaredMadeForKids": False}}


def _need(d: Path, quota, units: int) -> None:
    """Stop (not fail) when today's quota can't cover `units`; the run holds the upload."""
    if quota is not None and quota.left() < units:
        (d / QUOTA_WAIT).write_text(f"needs {units}, {quota.left()} left on {quota.day}\n")
        raise StepError(f"waiting for YouTube quota ({quota.left()} of {quota.limit} units left today)")


def upload_all(d: Path, yt, quota=None) -> dict:
    """Upload main + Short and the thumbnail, resuming from upload.json; a quiz has a
    Short only (no main, no custom thumbnail). With `quota`, never starts a call the
    day's remaining units can't cover."""
    meta = json.loads((d / "metadata.json").read_text())
    state_path = d / "upload.json"
    state = json.loads(state_path.read_text()) if state_path.exists() else {}
    for key in [k for k in ("main", "short") if k in meta]:
        if not state.get(key):
            _need(d, quota, COST_VIDEO)
            state[key] = yt.insert(d / "out" / f"{key}.mp4", build_body(meta[key], key == "short"))
            state_path.write_text(json.dumps(state))
            if quota is not None:
                quota.spend(COST_VIDEO)
    if "main" in meta and not state.get("thumbnail"):
        _need(d, quota, COST_THUMB)
        yt.set_thumbnail(state["main"], d / "out" / "thumb.png")
        state["thumbnail"] = True
        state_path.write_text(json.dumps(state))
        if quota is not None:
            quota.spend(COST_THUMB)
    return state


def load_credentials():
    from google.auth.exceptions import RefreshError
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials

    token = CONFIG / "youtube_token.json"
    if not token.exists():
        raise StepError(REAUTH)
    creds = Credentials.from_authorized_user_file(str(token), SCOPES)
    if not creds.valid:
        try:
            creds.refresh(Request())
        except RefreshError as e:
            raise StepError(f"{REAUTH} ({e})") from e
        token.write_text(creds.to_json())
    return creds


class YouTube:
    def __init__(self):
        from googleapiclient.discovery import build
        self.api = build("youtube", "v3", credentials=load_credentials(), cache_discovery=False)

    def insert(self, path: Path, body: dict) -> str:
        from googleapiclient.http import MediaFileUpload
        req = self.api.videos().insert(part="snippet,status", body=body,
                                       media_body=MediaFileUpload(str(path), chunksize=-1,
                                                                  resumable=True))
        resp = None
        while resp is None:
            _, resp = req.next_chunk()
        return resp["id"]

    def set_thumbnail(self, vid: str, path: Path) -> None:
        from googleapiclient.http import MediaFileUpload
        self.api.thumbnails().set(videoId=vid, media_body=MediaFileUpload(str(path))).execute()


def upload(d: Path, root: Path) -> dict:
    from googleapiclient.errors import HttpError
    quota = Quota(root)
    _need(d, quota, COST_THUMB)  # nothing at all left: don't even log in
    try:
        return upload_all(d, YouTube(), quota)
    except HttpError as e:
        raise StepError(f"YouTube API: {e}") from e


def auth() -> None:
    """One-time login. Headless machine: `ssh -L 8080:localhost:8080 <machine>` first,
    then open the printed URL in your local browser."""
    from google_auth_oauthlib.flow import InstalledAppFlow
    secret = CONFIG / "client_secret.json"
    if not secret.exists():
        raise SystemExit(f"missing {secret}: download the OAuth desktop client JSON there")
    creds = InstalledAppFlow.from_client_secrets_file(str(secret), SCOPES).run_local_server(
        port=8080, open_browser=False)
    token = CONFIG / "youtube_token.json"
    token.write_text(creds.to_json())
    token.chmod(0o600)
    print(f"saved {token}")
