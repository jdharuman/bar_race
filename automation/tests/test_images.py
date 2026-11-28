import json

import images

JPEG = b"\xff\xd8\xff\xe0" + b"0" * 100


def test_licence_rule_three():
    for ok in ("CC0", "Public domain", "PD-US", "CC BY 4.0", "CC BY-SA 3.0"):
        assert images.free_licence(ok, logo=False), ok
    for bad in ("Fair use", "CC BY-NC 2.0", "CC BY-ND 4.0", "All rights reserved", ""):
        assert not images.free_licence(bad, logo=False), bad
    assert images.free_licence("Public domain", logo=True)
    assert not images.free_licence("CC BY-SA 4.0", logo=True)  # logos: public domain only


def fake_fetch(files):
    """files: Commons filename -> licence."""
    def fetch(url, timeout=60, sleep=None):
        if "wbgetclaims" in url:
            return json.dumps({"claims": {"P18": [
                {"mainsnak": {"datavalue": {"value": f}}} for f in files]}}).encode()
        if "commons.wikimedia.org/w/api.php" in url:
            name = next(f for f in files if f.replace(" ", "+") in url or f in url)
            return json.dumps({"query": {"pages": {"1": {"imageinfo": [{
                "thumburl": f"https://upload/{name}", "mime": "image/jpeg",
                "descriptionurl": f"https://commons/{name}",
                "extmetadata": {"LicenseShortName": {"value": files[name]},
                                "Artist": {"value": "<a href='x'>Jane Doe</a>"}}}]}}}}).encode()
        return JPEG
    return fetch


def test_get_skips_non_free_and_records_credit(tmp_path, monkeypatch):
    monkeypatch.setattr(images, "_fetch", fake_fetch({"Nonfree.jpg": "Fair use",
                                                      "Free.jpg": "CC BY-SA 4.0"}))
    res = images.get("Q12495", tmp_path / "images" / "burj", item="Burj Khalifa")
    assert res["ok"] and res["path"].endswith("burj.jpg") and res["licence"] == "CC BY-SA 4.0"
    credits = (tmp_path / "images" / "credits.csv").read_text()
    assert "Burj Khalifa,Q12495,https://commons/Free.jpg,Jane Doe,CC BY-SA 4.0" in credits


def test_get_reports_no_free_picture(tmp_path, monkeypatch):
    monkeypatch.setattr(images, "_fetch", fake_fetch({"Nonfree.jpg": "CC BY-NC 2.0"}))
    res = images.get("Q1", tmp_path / "x")
    assert not res["ok"] and not (tmp_path / "x.jpg").exists()


def test_get_rejects_bad_ids(tmp_path):
    try:
        images.get("../etc", tmp_path / "x")
        assert False
    except ValueError:
        pass
