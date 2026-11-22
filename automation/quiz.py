"""Flag / capital quiz Shorts: a deterministic generator (no Claude, no web research).

A topics.csv row `quiz,"Guess the Flag: Europe #3"` becomes a video folder with
quiz.csv (10 questions), flags/, video.toml, metadata.json and research_notes.md.
The number picks a window of a fixed per-category order, so consecutive numbers
cover every country before any repeats; the title seeds wrong answers and order.
Data: quiz_data/ (Wikidata, CC0); flags: flagcdn.com (public domain).
"""
import csv
import json
import random
import re
import shutil
import urllib.request
from pathlib import Path

DATA = Path(__file__).resolve().parent / "quiz_data"
QUESTIONS = 10
FLAG_URL = "https://flagcdn.com/w640/{}.png"
REGIONS = {"europe": "Europe", "asia": "Asia", "africa": "Africa", "americas": "Americas",
           "oceania": "Oceania"}
FLAG_SETS = {"europe", "asia", "africa", "americas", "oceania and islands", "world mix",
             "easy mode", "hard mode", "look-alikes"}
CAPITAL_SETS = {"world", "europe", "asia", "africa", "indian states"}
TITLE = re.compile(r"^guess\s+the\s+(flag|capital)s?\s*:\s*(.+?)\s*#\s*(\d+)\s*$", re.I)


class QuizError(Exception):
    pass


def parse_title(title: str) -> tuple[str, str, int]:
    """'Guess the Flag: Europe #3' -> ('flag', 'europe', 3)."""
    m = TITLE.match(title.strip())
    if not m:
        raise QuizError(f"quiz title must look like 'Guess the Flag: Europe #3' (got {title!r})")
    kind, cat, n = m.group(1).lower(), " ".join(m.group(2).lower().split()), int(m.group(3))
    sets = FLAG_SETS if kind == "flag" else CAPITAL_SETS
    if cat not in sets or n < 1:
        raise QuizError(f"unknown {kind} quiz {m.group(2)!r}; one of: {', '.join(sorted(sets))}")
    return kind, cat, n


def countries() -> list[dict]:
    with (DATA / "countries.csv").open(newline="", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def _flag_entry(c: dict) -> dict:
    cap = c["capital"]
    fact = (f"Capitals: {cap}" if "/" in cap else f"Capital: {cap}") if cap else REGIONS[c["region"]]
    return {"key": c["iso2"], "image": c["iso2"], "prompt": "", "answer": c["name"],
            "fact": fact, "region": c["region"], "group": c["lookalike"]}


def _capital_entry(c: dict) -> dict:
    return {"key": c["iso2"], "image": c["iso2"], "prompt": c["name"], "answer": c["capital"],
            "fact": f"{c['capital']}, {c['name']}", "region": c["region"], "group": ""}


def pool(kind: str, cat: str) -> list[dict]:
    """Every possible question of a quiz set (also the source of wrong answers)."""
    if kind == "capital" and cat == "indian states":
        with (DATA / "india_states.csv").open(newline="", encoding="utf-8") as f:
            return [{"key": r["name"], "image": "", "prompt": r["name"], "answer": r["capital"],
                     "fact": f"{r['capital']}, {r['name']}", "region": "india", "group": ""}
                    for r in csv.DictReader(f)]
    cs = countries()
    if kind == "capital":
        cs = [c for c in cs if c["capital_quiz"] == "1"]
        if cat != "world":
            cs = [c for c in cs if c["region"] == cat]
        return [_capital_entry(c) for c in cs]
    pick = {
        "oceania and islands": lambda c: c["island"] == "1",
        "world mix": lambda c: True,
        "easy mode": lambda c: c["easy"] == "1",
        "hard mode": lambda c: c["easy"] == "0",
        "look-alikes": lambda c: bool(c["lookalike"]),
    }.get(cat, lambda c: c["region"] == cat)
    return [_flag_entry(c) for c in cs if pick(c)]


def wrong_answers(e: dict, all_entries: list[dict], rng: random.Random, hard: bool) -> list[str]:
    """Three plausible wrong answers: look-alike flags first (hard sets), then the same
    region, then anything in the set."""
    tiers = []
    if hard and e["group"]:
        tiers.append([x for x in all_entries if x["group"] == e["group"]])
    tiers.append([x for x in all_entries if x["region"] == e["region"]])
    tiers.append(all_entries)
    out = []
    for tier in tiers:
        names = sorted({x["answer"] for x in tier} - {e["answer"]} - set(out))
        rng.shuffle(names)
        out += names[:3 - len(out)]
        if len(out) == 3:
            return out
    raise QuizError(f"not enough wrong answers for {e['answer']}")


def build(title: str, n_questions: int = QUESTIONS) -> tuple[str, str, list[dict]]:
    kind, cat, n = parse_title(title)
    entries = pool(kind, cat)
    if len(entries) < n_questions:
        raise QuizError(f"{title}: only {len(entries)} possible questions")
    order = sorted(entries, key=lambda e: e["key"])
    random.Random(f"{kind}:{cat}").shuffle(order)  # fixed per set: #1, #2… walk through it
    start = (n - 1) * n_questions % len(order)
    chosen = [order[(start + i) % len(order)] for i in range(n_questions)]
    rng = random.Random(" ".join(title.lower().split()))
    rng.shuffle(chosen)
    hard = cat in ("look-alikes", "hard mode")
    qs = []
    for e in chosen:
        choices = [e["answer"], *wrong_answers(e, entries, rng, hard)]
        rng.shuffle(choices)
        qs.append({**e, "choices": choices, "correct": choices.index(e["answer"]) + 1})
    return kind, cat, qs


def fetch_flag(iso2: str, dest: Path, cache: Path) -> None:
    cached = cache / f"{iso2}.png"
    if not cached.exists():
        cache.mkdir(parents=True, exist_ok=True)
        req = urllib.request.Request(FLAG_URL.format(iso2), headers={"User-Agent": "barrace-quiz"})
        with urllib.request.urlopen(req, timeout=60) as r:
            data = r.read()
        if not data.startswith(b"\x89PNG"):
            raise QuizError(f"flag {iso2}: not a PNG")
        tmp = cached.with_suffix(".tmp")
        tmp.write_bytes(data)
        tmp.replace(cached)
    shutil.copyfile(cached, dest)


def _toml_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def generate(root: Path, d: Path, title: str, fetch=fetch_flag) -> None:
    """Write every input file of a quiz video into `d`."""
    kind, cat, qs = build(title)
    name = title.split(":", 1)[0].strip()
    sub = title.split(":", 1)[1].strip()
    flags = d / "flags"
    flags.mkdir(parents=True, exist_ok=True)
    for q in qs:
        if q["image"]:
            fetch(q["image"], flags / f"{q['image']}.png", root / "videos" / "flagcache")
    with (d / "quiz.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["image", "prompt", "a", "b", "c", "d", "answer", "fact"])
        for q in qs:
            w.writerow([f"flags/{q['image']}.png" if q["image"] else "", q["prompt"],
                        *q["choices"], q["correct"], q["fact"]])
    (d / "video.toml").write_text(
        f"style = \"quiz\"\ntitle = {_toml_str(name)}\nsubtitle = {_toml_str(sub)}\n"
        "reviewed = true  # generated from bundled, sourced data\n")
    what = "flags" if kind == "flag" else "capitals"
    ask = "Name the country from its flag" if kind == "flag" else "Name the capital"
    region = "Indian state" if cat == "indian states" else "country"
    desc = (f"{title} – {QUESTIONS} {what}, 3 seconds each. {ask} before the timer runs out "
            "and comment your score!\n\n"
            "Flags: flagcdn.com (public domain). Country and capital data: Wikidata (CC0).")
    tags = ["quiz", "flag quiz" if kind == "flag" else "capital quiz", f"guess the {kind}",
            "geography", "geography quiz", "trivia", f"{region} quiz", sub.split("#")[0].strip()]
    (d / "metadata.json").write_text(json.dumps({"short": {
        "title": f"{title} | Can You Get 10/10?", "description": desc, "tags": tags}}, indent=2))
    lines = [f"# {title}", "", "Generated by automation/quiz.py from automation/quiz_data "
             "(Wikidata, CC0). Flags: flagcdn.com (public domain). No estimated values.", ""]
    lines += [f"{i}. {q['prompt'] or q['image'].upper()} → {q['answer']} ({q['fact']})"
              for i, q in enumerate(qs, 1)]
    (d / "research_notes.md").write_text("\n".join(lines) + "\n")
