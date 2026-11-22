# Stock comparison: line-chart style — design

Date: 2026-11-22. Owner asks for a third video type: give 2–3 company names,
get a video comparing their share price history as an animated line chart.

## Decisions (owner)

- Measure: **value of $1,000 invested** at the start (split- and
  dividend-adjusted prices, i.e. dividends reinvested).
- Period: **since all were listed** — start at the youngest company's first
  full month of trading; run to the latest month.
- Data: **Yahoo Finance chart API** (no key; `adjclose`), cross-checked
  against a second source. Yahoo's API is unofficial; noted in
  `research_notes.md`.
- Formats: main (16:9) + Short (9:16), like the other styles.
- Our own look. The example Short the owner linked is a format hint only
  (CLAUDE.md rule 4).

## Pieces

### Renderer: generic `style = "lines"`

Reuses `data.csv` (one item per line, monthly `year` values), `Timeline`,
`events.csv`, intro/end cards, music and encoder. New `src/lines.rs`:

- `LineTracker` (per frame): visible time range `[start, t]`, x-axis end
  `max(t, start + min_span)` so the line heads sit at the right edge while
  the axis compresses; y-axis max eases up to 1.12 × the largest visible
  value (never below it), linear scale from 0.
- `LinePainter`: title block, faint date counter, horizontal gridlines with
  value ticks, year ticks, one stroked line per item in its colour (image
  colour, else palette), a head circle (image or initials) at the tip and a
  `Name  $12,345` label right of it; labels pushed apart vertically so they
  never overlap.
- `rebase = 1000` in `video.toml`: each line is drawn as
  `rebase × value(t) / value(start)`, where `start` is the first time the
  video shows (the Short may trim the start; the line still starts at
  $1,000). `data.csv` keeps the raw adjusted prices, so every number can be
  checked against the source.
- A line stops at its last data point (no ramp to zero as in the race).
- Fact captions (`events.csv`) sit top-left of the chart (landscape) or below
  it (Short). `Layout::caption_at` overrides the caption box for this.
- Thumbnail: last frame with the big title overlay.
- `validate`: 1–6 items; with `rebase`, values must be > 0 and every item
  must have data at the first month (so all lines start at $1,000).

### Research: topic style `stocks`

`topics.csv` / Telegram: `style = stocks`, title like
`Apple vs Microsoft vs Nvidia`. The research skill gets a `stocks` section:
find each company's main listing ticker, fetch monthly `adjclose` from
`https://query1.finance.yahoo.com/v8/finance/chart/<T>?range=max&interval=1mo`,
trim all series to the common start, write `data.csv`
(`name=company, category=ticker, value=adjclose, year=YYYY-MM`) and
`video.toml` with `style = "lines"`, `rebase = 1000`, a subtitle saying what
is measured, and a pace giving a 2–4 minute main video. Cross-check 2–3
month-end closes per company against a second source (split factors
explain differences in raw closes). Description ends with "Not investment
advice."

### Automation

- `daily.py`: `STYLES` gains `stocks`.
- Telegram bot: third style button **Stocks**; the title question becomes
  "Which companies? (2–3, e.g. Apple vs Microsoft vs Nvidia)", requiring
  2–3 names.

## Out of scope (later)

Log-scale y-axis; crypto / indices; more than 6 lines; Short-only output.
