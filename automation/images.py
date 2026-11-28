"""Find a real, freely licensed picture for a chart item (person, building, car,
painting, city, animal...) and download it into a video folder.

Used by the research agent (`uv run --project automation automation/images.py ...`):

    images.py search "Burj Khalifa"                      # candidates: Q-id, label, description
    images.py get Q12495 <dir>/images/burj-khalifa.jpg   # picture of that item

`get` takes the item's Wikidata image (P18), or with --logo its logo (P154, public
domain only), checks the licence on Wikimedia Commons in code (CLAUDE.md rule 3:
CC0, public domain, CC BY, CC BY-SA), downloads a 500 px thumbnail (a standard Wikimedia size) and appends the
credit to images/credits.csv next to it. Prints one JSON line; exit 1 = no free picture.
"""
import argparse
import csv
import html
import json
import re
import sys
import urllib.parse
import urllib.error
import urllib.request
import time
from pathlib import Path

UA = {"User-Agent": "barrace-images/1.0 (data-visualization tool; Wikimedia API client)"}
WIKIDATA = "https://www.wikidata.org/w/api.php"
COMMONS = "https://commons.wikimedia.org/w/api.php"
FREE = re.compile(r"^(cc0|public domain|pd\b|pd-|cc by(-sa)? \d)", re.I)
CREDIT_COLS = ["file", "item", "qid", "commons_page", "author", "licence"]


def _fetch(url: str, timeout: int = 60, sleep=time.sleep) -> bytes:
    """GET with Wikimedia's rate limits in mind: on 429/503 wait (Retry-After) and retry."""
    for attempt in range(5):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=timeout) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code not in (429, 503) or attempt == 4:
                raise
            wait = e.headers.get("Retry-After", "")
            sleep(min(int(wait), 60) if wait.isdigit() else 5 * 2 ** attempt)
    raise RuntimeError("unreachable")


def _get(url: str, params: dict) -> dict:
    return json.loads(_fetch(f"{url}?{urllib.parse.urlencode({**params, 'format': 'json'})}", 30))


def search(name: str, n: int = 6) -> list[dict]:
    r = _get(WIKIDATA, {"action": "wbsearchentities", "search": name, "language": "en",
                        "type": "item", "limit": n})
    return [{"qid": e["id"], "label": e.get("label", ""), "description": e.get("description", "")}
            for e in r.get("search", [])]


def entity_files(qid: str, prop: str) -> list[str]:
    r = _get(WIKIDATA, {"action": "wbgetclaims", "entity": qid, "property": prop})
    return [c["mainsnak"]["datavalue"]["value"] for c in r.get("claims", {}).get(prop, [])
            if c.get("mainsnak", {}).get("datavalue")]


def free_licence(licence: str, logo: bool) -> bool:
    """Rule 3: photos CC0/PD/CC BY/CC BY-SA; logos public domain only."""
    lic = licence.strip()
    if logo:
        return bool(re.match(r"^(cc0|public domain|pd\b|pd-)", lic, re.I))
    return bool(FREE.match(lic))


def commons_info(filename: str, width: int = 500) -> dict:
    r = _get(COMMONS, {"action": "query", "titles": f"File:{filename}", "prop": "imageinfo",
                       "iiprop": "url|extmetadata|mime", "iiurlwidth": width})
    page = next(iter(r["query"]["pages"].values()))
    info = (page.get("imageinfo") or [{}])[0]
    meta = info.get("extmetadata", {})
    text = lambda k: html.unescape(re.sub(r"<[^>]+>", "", meta.get(k, {}).get("value", ""))).strip()
    return {"thumb": info.get("thumburl", ""), "mime": info.get("mime", ""),
            "page": info.get("descriptionurl", ""), "licence": text("LicenseShortName"),
            "author": " ".join(text("Artist").split())[:120] or "unknown"}


def get(qid: str, out: Path, logo: bool = False, item: str = "") -> dict:
    if not re.fullmatch(r"Q\d+", qid):
        raise ValueError(f"not a Wikidata id: {qid}")
    for f in entity_files(qid, "P154" if logo else "P18"):
        info = commons_info(f)
        if not info["thumb"] or not free_licence(info["licence"], logo):
            continue
        data = _fetch(info["thumb"])
        if not data[:4] in (b"\x89PNG", b"\xff\xd8\xff\xe0", b"\xff\xd8\xff\xe1", b"\xff\xd8\xff\xdb",
                            b"\xff\xd8\xff\xee", b"\xff\xd8\xff\xe2"):
            continue  # not a PNG/JPEG thumbnail
        out = out.with_suffix(".png" if data[:4] == b"\x89PNG" else ".jpg")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(data)
        credit = {"file": out.name, "item": item or qid, "qid": qid, "commons_page": info["page"],
                  "author": info["author"], "licence": info["licence"]}
        log = out.parent / "credits.csv"
        new = not log.exists()
        with log.open("a", newline="") as fh:
            w = csv.DictWriter(fh, CREDIT_COLS)
            if new:
                w.writeheader()
            w.writerow(credit)
        return {"ok": True, "path": str(out), **credit}
    return {"ok": False, "qid": qid, "reason": "no image with a free licence on this item"}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("search")
    s.add_argument("name")
    g = sub.add_parser("get")
    g.add_argument("qid")
    g.add_argument("out", type=Path, help="target path; the extension is set from the file type")
    g.add_argument("--logo", action="store_true", help="the item's logo (public domain only)")
    g.add_argument("--item", default="", help="chart name, for the credits file")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "search":
            for c in search(a.name):
                print(json.dumps(c, ensure_ascii=False))
            return 0
        res = get(a.qid, a.out, a.logo, a.item)
    except Exception as e:  # network or API trouble: say so, don't crash the agent's loop
        res = {"ok": False, "reason": f"{type(e).__name__}: {e}"}
    print(json.dumps(res, ensure_ascii=False))
    return 0 if res.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
