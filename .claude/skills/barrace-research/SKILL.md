---
name: barrace-research
description: Research one topic from a video folder's topic.json into barrace input files (data.csv or pillars.csv, video.toml, events.csv, images, metadata.json, research_notes.md) that pass `barrace validate`. Used headless by automation/daily.py research.
---

# barrace research

You turn one topic into a finished, validated video folder. Nobody reviews
the data before render: the owner watches the private video afterwards. So
the data must be right, sourced and complete. Follow the hard rules in the
repo's `CLAUDE.md` above everything here.

Work only inside the video folder you were given (`<dir>`). Never render,
upload or touch files outside `<dir>`: you can't write elsewhere, and a
change to the repo's tool files fails the run.

**Keep it short.** Aim to finish in 5–10 minutes (hard stop at 20). Use one
good primary dataset plus one cross-check. Don't browse widely, don't start
subagents, and don't re-check what already matches. Write data with one
script, run it once, then fix only what `validate` reports.

**Commands you can run:** `curl`, `uv run …` (Python, for all data
processing: `uv run --with pandas python <dir>/scripts/x.py`), `yt-dlp`,
`ls`, `mkdir`, `unzip`, `file`, `head`, `wc`, and `<barrace> validate`. To
copy, move or delete files, use Python via `uv run`. Plus the Read, Write,
Edit, Glob, Grep, WebSearch and WebFetch tools.
Run **one plain command per Bash call** with absolute paths: no `cd`, `;`,
`&&`, pipes into other tools, loops or heredocs (those are refused). Create
files, scripts included, with the Write tool.

**Can't do it honestly?** If no trustworthy source covers the topic (or
most of its values would be guesses), don't invent data. Write the reason in
`research_notes.md` and stop without writing `metadata.json`. The run then
fails visibly and the owner can rephrase the topic.

## 1. Read the topic

`<dir>/topic.json`:

```json
{"row": {"style": "race|pillars", "title": "...", "youtube_url": "...", "comment": "..."},
 "redo_of": "/path/to/previous/folder or null", "picked_at": "..."}
```

- **Redo** (`comment` non-empty): the folder already holds the previous
  attempt (copied from `redo_of`, if found), with its notes renamed to
  `research_notes.prev.md`. Read `comment`, the owner's list of mistakes,
  fix exactly those, keep everything else, and write a new
  `research_notes.md` (carry over what's still true from the `.prev` file)
  with a `## Redo fixes` section listing each fix. **The run fails without
  that section.** If the folder is empty, research from scratch and also
  honour the comment (still with `## Redo fixes`).
- **YouTube link:** read metadata only:
  `yt-dlp --js-runtimes node --dump-json --skip-download <url>` → title,
  description, duration, chapters. Use them to learn the topic, the set of
  items and the time range. Never download the video, frames, thumbnail or
  audio. The numbers come from your own sources (step 2).
- Only a link, no title: derive the topic from the video's title.

## 2. Find the data

- Prefer whole datasets from primary sources, downloaded with `curl`:
  World Bank API (`https://api.worldbank.org/v2/country/all/indicator/<ID>?format=json&per_page=20000&date=1960:2030`),
  Our World in Data grapher CSVs (`https://ourworldindata.org/grapher/<slug>.csv`),
  IMF, UN, official statistics offices, Wikipedia tables (`WebFetch`) with
  their cited sources.
- Save raw downloads in `<dir>/raw/` and any processing script in
  `<dir>/scripts/`. Run scripts with `uv run --with pandas python <script>`.
- **Run the series to the latest available period** (this year, or the
  latest year the source has published), even if the reference video stopped
  earlier.
- Drop aggregates (World, regions, income groups) from country data.
- Cross-check the latest values of the top items against a second source.
  Big disagreements: prefer the primary source and note it.
- Gaps: interpolate only short gaps inside a series and mark those rows
  `estimated=true`. Keep estimated rows under ~15% of the total; if more are
  needed, choose a better source or a shorter range.

## 3. Write the data file

Exact formats are in `README.md` (sections "data.csv", "video.toml",
"events.csv", "Pillar walk"). Key points:

**race → `data.csv`** `name,category,image,year,value,source,estimated`
- Raw units (dollars not billions, people not millions) so digits spin.
- Include every item that is ever in the top ~15 at any time, with all its
  years, so bars enter and leave correctly.
- `year`: `1999`, or `1999-02`/`1999-Q2` when monthly/quarterly data exists
  and makes the race richer.
- `category`: continent/region (or sector for companies).
- `source`: the dataset URL.

**pillars → `pillars.csv`** `label,name,value,detail,image,source,estimated`
- One row per record holder in display (chronological) order; `label` is
  the year/season, `detail` a short context (team, country).
- 8–80 rows. More than 80: keep the most recent 80.

## 4. Images (`<dir>/images/`)

- **Country flags:** `curl -sL https://flagcdn.com/w320/<iso2>.png -o images/<iso2>.png`
  (flags are public domain). Use them for country items and for people's
  nationality only when there's no free photo and the flag helps.
- **People, buildings, companies:** only Wikimedia Commons files with a free
  licence. Check with
  `https://commons.wikimedia.org/w/api.php?action=query&titles=File:<name>&prop=imageinfo&iiprop=url|extmetadata&iiurlwidth=400&format=json`
  and accept only `LicenseShortName` of CC0, Public domain, CC BY x.x or
  CC BY-SA x.x. Download the `thumburl`. Record file page, author and licence
  for each image. Logos: only when Commons says public domain (e.g.
  text-only logos).
- No free image → leave `image` empty. Never use images from news sites,
  Google Images, fan sites or the reference video.
- Check each download really is an image (`file images/x.png`).

## 5. video.toml

```toml
title = "<short, ≤ 45 chars>"
subtitle = "<what is measured> (<main source>)"
unit_prefix = "$"     # or ""
unit_suffix = ""      # e.g. " m", " people" — leading space
number_style = "full" # money/population; "compact" only for huge, noisy values
reviewed = true       # the owner reviews the finished video instead
```

- race: aim for a 3–5 minute main video. Leave the default pace unless the
  range is long; then set `seconds_per_year` or `[[pace]]` sections (fast
  for sparse early eras, slower for recent decades).
- pillars: `style = "pillars"`; `seconds_per_pillar` 2.5 (2.0 above 60 rows).
- Don't set `music`; the renderer picks a bundled CC0 track.

## 6. events.csv (race only, optional but preferred)

3–8 fact captions tied to real events in the data (`from,to,headline,big,caption`),
e.g. `2008,2010,Financial crisis,-4%,US GDP drop`. Each fact must be
supported by a source listed in `research_notes.md`. Spans 2–6 years; no
overlaps.

## 7. metadata.json

```json
{
  "main":  {"title": "...", "description": "...", "tags": ["..."]},
  "short": {"title": "...", "description": "...", "tags": ["..."]}
}
```

- Titles: hooky but true, ≤ 70 chars, with the year range
  (e.g. "Largest Economies in the World 1960–2025").
- Description: 1–2 line hook, what is measured and the period, then
  `Sources:` (one per line, URLs), then `Photo credits:` (author, licence,
  Commons URL) when any CC BY / CC BY-SA image is used, then
  "* = estimated values" if any. English.
- 10–15 lowercase tags. The upload step adds `#Shorts` itself.

## 8. research_notes.md

Sources (URL + what was taken), cross-checks, every estimated value and why,
image list with licence, redo fixes, anything a viewer might dispute.

## 9. Validate until clean

Run `<barrace> validate <dir>` (path given in your prompt). Fix every
`error`. Read the warnings and fix what you can. Done only when it reports no
errors and `metadata.json` exists.
