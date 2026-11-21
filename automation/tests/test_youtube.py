import json
from pathlib import Path

import youtube

META = {"main": {"title": "Largest Economies 1960-2025", "description": "Sources: World Bank",
                 "tags": ["gdp", "economy"]},
        "short": {"title": "Largest Economies", "description": "GDP race", "tags": ["gdp"]}}


def test_body_private_and_category():
    b = youtube.build_body(META["main"], short=False)
    assert b["status"] == {"privacyStatus": "private", "selfDeclaredMadeForKids": False}
    assert b["snippet"]["categoryId"] == "27"
    assert b["snippet"]["title"] == "Largest Economies 1960-2025"


def test_short_body_has_shorts_tag():
    b = youtube.build_body(META["short"], short=True)
    assert "#Shorts" in b["snippet"]["title"]
    assert "#Shorts" in b["snippet"]["description"]


def test_long_title_truncated():
    b = youtube.build_body({"title": "x" * 150, "description": "", "tags": []}, short=False)
    assert len(b["snippet"]["title"]) == 100


def test_tags_capped_500():
    b = youtube.build_body({"title": "t", "description": "", "tags": ["abcdefghij"] * 80},
                           short=False)
    assert sum(len(t) + 1 for t in b["snippet"]["tags"]) <= 500


class FakeYT:
    def __init__(self):
        self.inserted, self.thumbs = [], []

    def insert(self, path, body):
        self.inserted.append((path.name, body))
        return f"id-{path.stem}"

    def set_thumbnail(self, vid, path):
        self.thumbs.append(vid)


def video_dir(tmp_path):
    (tmp_path / "out").mkdir()
    for f in ("main.mp4", "short.mp4", "thumb.png"):
        (tmp_path / "out" / f).write_text("x")
    (tmp_path / "metadata.json").write_text(json.dumps(META))
    return tmp_path


def test_upload_resumes(tmp_path):
    d = video_dir(tmp_path)
    (d / "upload.json").write_text(json.dumps({"main": "m1"}))
    yt = FakeYT()
    out = youtube.upload_all(d, yt)
    assert [n for n, _ in yt.inserted] == ["short.mp4"]
    assert yt.thumbs == ["m1"]
    assert out == {"main": "m1", "short": "id-short", "thumbnail": True}
    assert json.loads((d / "upload.json").read_text()) == out


def test_upload_never_public(tmp_path):
    yt = FakeYT()
    youtube.upload_all(video_dir(tmp_path), yt)
    assert len(yt.inserted) == 2
    assert all(b["status"]["privacyStatus"] == "private" for _, b in yt.inserted)
