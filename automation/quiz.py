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


# Display names, in rotation order for generated titles.
SET_NAMES = {("flag", "europe"): "Europe", ("flag", "asia"): "Asia", ("flag", "africa"): "Africa",
             ("flag", "americas"): "Americas", ("capital", "world"): "World",
             ("flag", "world mix"): "World Mix", ("flag", "easy mode"): "Easy Mode",
             ("capital", "europe"): "Europe", ("flag", "oceania and islands"): "Oceania and Islands",
             ("flag", "hard mode"): "Hard Mode", ("capital", "asia"): "Asia",
             ("flag", "look-alikes"): "Look-Alikes"}
# Capital quizzes Africa and Indian States drew single-digit views (2026-11-27): they stay
# valid titles (requests, old rows) but are never generated.
RECENT = 4  # `used` is oldest first: none of the last 4 sets comes up again
FLAGS_PER_CAPITAL = 3  # generated quizzes: about 3 flag quizzes for every capital quiz


def next_title(used: list[str]) -> str:
    """The next quiz title after `used`: the set whose pool has been covered the least
    (so big sets come up more often), numbered one past its highest number so far.
    A capital quiz only comes up when capitals are below 1 in FLAGS_PER_CAPITAL + 1,
    so a night's quizzes are mostly flags with the odd capital, not a run of capitals."""
    count, top, order = {}, {}, []
    for t in used:
        try:
            kind, cat, n = parse_title(t)
        except QuizError:
            continue
        count[(kind, cat)] = count.get((kind, cat), 0) + 1
        top[(kind, cat)] = max(top.get((kind, cat), 0), n)
        order.append((kind, cat))
    recent = set(order[-RECENT:])
    caps = sum(v for k, v in count.items() if k[0] == "capital")
    flags = sum(v for k, v in count.items() if k[0] == "flag")
    kind = "capital" if caps * FLAGS_PER_CAPITAL < flags else "flag"
    sets = [k for k in SET_NAMES if k[0] == kind]
    sets = [k for k in sets if k not in recent] or sets  # no set twice in a row
    key = min(sets, key=lambda k: count.get(k, 0) * QUESTIONS / len(pool(*k)))
    return f"Guess the {key[0].title()}: {SET_NAMES[key]} #{top.get(key, 0) + 1}"


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


# Translated quiz titles (YouTube shows them to viewers with that language set).
# lang: (flag title, capital title, ending)
LOCAL_TITLE = {
    "de": ("Errate die Flagge", "Errate die Hauptstadt", "Schaffst du 10/10?"),
    "fr": ("Devine le drapeau", "Devine la capitale", "Peux-tu faire 10/10 ?"),
    "it": ("Indovina la bandiera", "Indovina la capitale", "Riesci a fare 10/10?"),
    "es": ("Adivina la bandera", "Adivina la capital", "¿Puedes sacar 10/10?"),
    "nl": ("Raad de vlag", "Raad de hoofdstad", "Haal jij 10/10?"),
    "pl": ("Zgadnij flagę", "Zgadnij stolicę", "Dasz radę 10/10?"),
}
LOCAL_SET = {  # category -> names in LOCAL_TITLE's language order (de, fr, it, es, nl, pl)
    "europe": ("Europa", "Europe", "Europa", "Europa", "Europa", "Europa"),
    "asia": ("Asien", "Asie", "Asia", "Asia", "Azië", "Azja"),
    "africa": ("Afrika", "Afrique", "Africa", "África", "Afrika", "Afryka"),
    "americas": ("Amerika", "Amériques", "Americhe", "Américas", "Amerika", "Ameryki"),
    "world": ("Welt", "Monde", "Mondo", "Mundo", "Wereld", "Świat"),
    "world mix": ("Weltmix", "Mix mondial", "Mix mondiale", "Mezcla mundial", "Wereldmix",
                  "Mix świata"),
    "easy mode": ("Leicht", "Facile", "Facile", "Fácil", "Makkelijk", "Łatwy poziom"),
    "hard mode": ("Schwer", "Difficile", "Difficile", "Difícil", "Moeilijk", "Trudny poziom"),
    "oceania and islands": ("Ozeanien und Inseln", "Océanie et îles", "Oceania e isole",
                            "Oceanía e islas", "Oceanië en eilanden", "Oceania i wyspy"),
    "look-alikes": ("Verwechslungsgefahr", "Sosies", "Simili", "Parecidas", "Dubbelgangers",
                    "Podobne"),
    "indian states": ("Indische Bundesstaaten", "États indiens", "Stati indiani",
                      "Estados de la India", "Indiase deelstaten", "Stany Indii"),
}


def local_titles(title: str) -> dict:
    """metadata.json `localizations` for a quiz title (youtube.build_body adds #Shorts)."""
    kind, cat, n = parse_title(title)
    names = LOCAL_SET.get(cat)
    if not names:
        return {}
    out = {}
    for i, (lang, (flag, capital, end)) in enumerate(LOCAL_TITLE.items()):
        sep = " : " if lang == "fr" else ": "
        out[lang] = {"title": f"{flag if kind == 'flag' else capital}{sep}{names[i]} #{n} | {end}"}
    return out


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
        "title": f"{title} | Can You Get 10/10?", "description": desc, "tags": tags,
        "localizations": local_titles(title)}}, indent=2, ensure_ascii=False))
    lines = [f"# {title}", "", "Generated by automation/quiz.py from automation/quiz_data "
             "(Wikidata, CC0). Flags: flagcdn.com (public domain). No estimated values.", ""]
    lines += [f"{i}. {q['prompt'] or q['image'].upper()} → {q['answer']} ({q['fact']})"
              for i, q in enumerate(qs, 1)]
    (d / "research_notes.md").write_text("\n".join(lines) + "\n")
