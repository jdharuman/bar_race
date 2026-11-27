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
{"row": {"style": "race|pillars|stocks|compare", "title": "...", "youtube_url": "...", "comment": "...",
         "look": "|light|neon", "theme": "|sports|india|economy|earth|tech|history|people"},
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

### stocks (style `stocks`): price history of 2–3 companies

The title names the companies (`Apple vs Microsoft vs Nvidia`). The video
shows what $1,000 invested in each would be worth, from the month the
youngest of them was listed to the latest month. Do this instead of the
general steps above, then continue with step 3.

1. **Tickers:** `curl -s -A "Mozilla/5.0" "https://query1.finance.yahoo.com/v1/finance/search?q=<name>&quotesCount=5"`
   → the company's main common-stock listing (`quoteType` EQUITY). Use one
   currency for all lines: US listings or US ADRs in USD. A company with no
   USD listing: use its home listing only if all others share that currency,
   else stop (see "Can't do it honestly?").
2. **Prices:** for each ticker
   `curl -s -A "Mozilla/5.0" "https://query1.finance.yahoo.com/v8/finance/chart/<T>?period1=0&period2=<now unix>&interval=1mo&events=split,div" -o <dir>/raw/<T>.json`.
   - Use `period1=0&period2=…`, **not** `range=max`: `range=max` silently
     returns quarterly bars for long histories. Check
     `meta.dataGranularity == "1mo"`.
   - Use `indicators.adjclose` (split- and dividend-adjusted = dividends
     reinvested). Skip null values.
   - Month of a bar: `datetime.fromtimestamp(ts + 2*86400, tz=UTC)` (bars are
     stamped at local midnight; a fixed UTC offset puts summer bars in the
     previous month). The last bar may be today's date in the current month:
     keep one value per month, the latest.
   - Start = the latest first month among the companies; drop earlier rows
     so every line starts in that month.
3. **Cross-check** each company against a second source (e.g. Nasdaq
   historical prices or companiesmarketcap.com via WebFetch): 2–3 month-end
   closes, compared with Yahoo's unadjusted `close` (adjclose differs by the
   dividend factor). List the splits from `events.splits` in the notes. A
   move of more than ±30% in the last 3 months: confirm it with a second
   source and say so in the notes.
4. Yahoo's chart API is unofficial; name it as the source and note that.

### compare (style `compare`): 2–3 things over time, as a line chart

The title names what is compared and the measure (`Messi vs Ronaldo career
goals`, `Tamil Nadu vs Karnataka GDP`). Use the general steps above (primary
sources, cross-check, no invented values) and write `data.csv` with one
`name` per line (2–3 names, at most 6), the same unit for all, yearly
(`2005`) or monthly (`2005-03`) `year` values. Choose the measure that tells
the story honestly: cumulative totals for careers ("career goals"), levels
for economies (GDP, population). Use `rebase` only for growth comparisons
("growth since 2000, 2000 = 100": `rebase = 100`). Every line must have data
for the whole period, or say in the notes why one starts later.

`theme` (any style) is the weekly theme/playlist; it needs no extra work.

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

**stocks → `data.csv`** (rendered with `style = "lines"`)
- One row per company and month: `name` = short company name (`Apple`),
  `category` = ticker, `image` = logo only if Commons says public domain,
  else empty (initials), `year` = `YYYY-MM`, `value` = adjclose (6
  significant digits), `source` = `https://finance.yahoo.com/quote/<T>/history`,
  `estimated` = false. No gap filling: a missing month stays missing.

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
- compare: `style = "lines"`, title = the comparison, subtitle = the measure
  and source (`Career goals for club and country (Transfermarkt)`), no
  `rebase` unless it is a growth comparison; `look = "neon"` when
  `row.look` is `neon`. Pace for a 2–4 minute main video.
- stocks: `style = "lines"`, `rebase = 1000`, `unit_prefix = "$"`,
  `number_style = "full"`, title = the companies (`Apple vs Microsoft vs Nvidia`),
  `subtitle = "Value of $1,000 invested in {start}, dividends reinvested (Yahoo Finance)"`.
  Keep `{start}` literally: the renderer fills in the first month each video
  shows (the Short may start later). **Indian (NSE) stocks** are in rupees:
  `unit_prefix = "₹"`, `rebase = 100000`, subtitle
  `"Value of ₹1 lakh invested in {start}, dividends reinvested (Yahoo Finance)"`
  (tickers end in `.NS`; never mix ₹ and $ in one video). **European
  stocks** whose main listings are all in euros (Xetra `.DE`, Euronext `.PA`
  `.AS` `.MI` `.MC`…): use those listings, `unit_prefix = "€"`, `rebase = 1000`,
  subtitle `"Value of €1,000 invested in {start}, dividends reinvested (Yahoo Finance)"`,
  and titles with € ("€1,000 in BMW vs Mercedes-Benz vs Volkswagen"); never
  mix € with another currency in one video. If `row.look` is `neon`, add
  `look = "neon"` (black background, glowing lines); otherwise leave `look` out. Main video 2–4 min:
  `seconds_per_year` = 180 / years, between 4 and 12.
- Don't set `music`; the renderer picks a bundled CC0 track.

## 6. events.csv (race and stocks, optional but preferred)

3–8 fact captions tied to real events in the data (`from,to,headline,big,caption`),
e.g. `2008,2010,Financial crisis,-4%,US GDP drop`. Each fact must be
supported by a source listed in `research_notes.md`. Spans 2–6 years; no
overlaps.

stocks: 2–5 captions with **month** dates, so a caption never shows before
its move: `from` = the first month of the move, `to` = its last month plus
about 6 months (may run past the data's end; it then stays up at the end).
E.g. `2025-12,2027-05,Coke's big run,+41%,Coca-Cola rose 41% Dec 2025–Nov 2026`.
Compute every `big` number from `data.csv`; keep `caption` under ~60
characters.

## 7. metadata.json

```json
{
  "main":  {"title": "...", "description": "...", "tags": ["..."]},
  "short": {"title": "...", "description": "...", "tags": ["..."],
            "localizations": {"de": {"title": "..."}, "fr": {"title": "..."},
                              "it": {"title": "..."}, "es": {"title": "..."},
                              "nl": {"title": "..."}, "pl": {"title": "..."}}}
}
```

- `short.localizations`: the Short title translated into German, French,
  Italian, Spanish, Dutch and Polish (YouTube shows it to viewers using that
  language). Same meaning, numbers and years as the English title, natural
  wording, names in their usual local spelling (e.g. "Deutschland",
  "Allemagne"), ≤ 70 chars, no `#Shorts`. Titles only; the description stays
  English. If unsure of a translation, leave that language out.

- Titles: hooky but true, ≤ 70 chars, with the year range
  (e.g. "Largest Economies in the World 1960–2025").
- Description: 1–2 line hook, what is measured and the period, then
  `Sources:` (one per line, URLs), then `Photo credits:` (author, licence,
  Commons URL) when any CC BY / CC BY-SA image is used, then
  "* = estimated values" if any. English.
- 10–15 lowercase tags. The upload step adds `#Shorts` itself.
- stocks: main title like "If You Invested $1,000 in Apple, Microsoft or
  Nvidia (1999–2026)"; the **Short** title and description must not name the
  start year (the Short may start later), e.g. "$1,000 in Apple vs Microsoft
  vs Nvidia". Both descriptions end with "Past performance does not predict
  future returns. Not investment advice."

## 8. research_notes.md

Sources (URL + what was taken), cross-checks, every estimated value and why,
image list with licence, redo fixes, anything a viewer might dispute.

## 9. Validate until clean

Run `<barrace> validate <dir>` (path given in your prompt). Fix every
`error`. Read the warnings and fix what you can. Done only when it reports no
errors and `metadata.json` exists.
