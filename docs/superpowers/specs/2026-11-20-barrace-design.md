# barrace — weekly bar-chart-race video pipeline

## Intent

Produce one data-visualization video per week (in the style of Flourish
"bar chart race" videos, e.g. Global Stats on YouTube) with an owned, open
tool. The only human creative step is **picking the topic**; the human also
**reviews the data table** before rendering. Everything else is automated.

Success = topic in Monday → reviewed CSV → a finished 16:9 video, a vertical
Short, thumbnail, title/description/tags, scheduled on YouTube, in under
~30 minutes of human time per week.

## Goals

- Rust renderer: CSV + config → smooth 60 fps bar chart race MP4 with music.
- Two outputs from the same data: 16:9 1920×1080 main video, 9:16 1080×1920 Short.
- Research agent (Option B): topic → CSV where every value is marked sourced
  (with URL) or estimated. Human reviews/edits the CSV before render.
- Publish kit: thumbnail PNG, title, description (with sources), tags,
  upload to YouTube.

## Non-goals (v1)

- GUI editor or live browser preview.
- Other chart types (line race, map, pie).
- Voiceover / narration, subtitles, multiple languages.
- TikTok / Instagram upload (Short MP4 can be uploaded by hand).

## Anti-goals

- Shipping unreviewed numbers: the render step refuses a CSV that has not
  been marked reviewed.
- Copying another channel's branding, intro, fonts or look. Own visual identity.
- Using copyrighted music. Only tracks from a local royalty-free folder.

## Constraints

- Linux box; `ffmpeg` installed; Rust nightly 1.94 installed; Python via
  `uv` (system Python is 3.8, too old — use uv-managed 3.12).
- Open-source dependencies only.
- Render time for the 4-min 16:9 video should be a few minutes, not tens.

## Approaches considered

| | Approach | Trade-off |
|---|---|---|
| **A (picked)** | Rust renderer: draw frames with `tiny-skia` + `cosmic-text`, pipe raw RGBA to `ffmpeg` | Fast, one binary, fully open, both aspect ratios = two layouts. Most up-front work for polish (easing, labels, text). |
| B | Web renderer (Canvas/D3 or Remotion) + headless Chrome capture | Quickest Flourish look, live preview. Slow renders, heavy stack, Remotion license not fully open. |
| C | Python (`bar_chart_race`/matplotlib) | Least code, but unmaintained, dated look, slow. |

Picked A: user asked for Rust; weekly batch favours fast reliable renders;
the style is designed once, so live preview matters less.

## Architecture

```
topic ─► research/ (Python+LLM) ─► data.csv + video.toml
                                        │  human reviews, sets reviewed = true
                                        ▼
                                 barrace (Rust) ─► main.mp4, short.mp4, thumb.png
                                        ▼
                                 publish/ (Python) ─► metadata + YouTube upload
```

Each week lives in its own folder: `weeks/2026-11-23-largest-economies/`
holding `data.csv`, `video.toml`, `images/`, and `out/`.

## Data contract

### `data.csv` (long format, one row per item per year)

```csv
name,category,image,year,value,source,estimated
USA,North America,images/us.png,1960,543.3,https://data.worldbank.org/...,false
China,Asia,images/cn.png,1960,59.7,https://data.worldbank.org/...,false
```

- `name` unique item id/label. `category` drives bar colour.
- `image` optional relative path; missing → coloured circle with initials.
- `year` integer. `value` float. Missing years for an item are allowed.
- `source` URL or empty; `estimated` true/false.

### `video.toml`

```toml
title = "Largest Economies in the World"
subtitle = "Nominal GDP, billions USD"
unit_prefix = "$"
unit_suffix = "B"
top_n = 10                 # bars shown (Short uses top_n_short, default 8)
seconds_per_year = 3.5     # main video pacing
short_seconds = 58         # Short total length; pacing derived
music = "music/epic-01.mp3" # optional; else random from music/
reviewed = false           # human flips to true after checking data
[colors]                   # optional category → hex; else auto palette
Asia = "#e4572e"
```

`barrace validate` checks the CSV/TOML (columns, types, duplicates, image
paths exist, `reviewed`) and prints a summary: items, year range, % estimated.

## Renderer behaviour (Rust)

- **Timeline:** intro title card (3 s) → race → hold final frame (3 s) →
  end card with sources summary (4 s).
- **Interpolation:** values between yearly data points interpolated
  linearly per frame; an item's value before its first / after its last
  data point = absent (bar slides out). Gaps inside an item's range are
  interpolated across.
- **Ranking:** computed per frame from interpolated values; bar *position*
  eases toward its rank slot (smoothed, ~0.4 s) so overtakes glide.
- **Bar:** colour by category, rounded end, image at bar tip, name label,
  value label counting up with unit, thousands separators.
- **Axis:** x-scale eases to current max; light gridlines with tick values.
- **Overlay:** big year counter (bottom-right on 16:9, bottom on 9:16),
  title + subtitle top-left, small channel watermark.
- **Layouts:** `landscape` (1920×1080, top_n) and `portrait`
  (1080×1920, top_n_short, larger text, title at top, safe margins for
  Shorts UI). Same scene, different layout constants.
- **Short timeline:** 1.5 s title card, then the race, then a 1.5 s hold.
  No end card. Total = `short_seconds`. If the pace would drop below
  0.8 s/year, the Short starts at a later year so pace stays readable.
- **Estimated values:** the value label gets a small `*`, and the end card
  (main video) says "* estimated" with the share of estimated values.
- **Channel identity:** `channel.toml` at the repo root holds the channel
  name, optional logo path and brand colours. The watermark, cards and
  thumbnail use it. No channel identity exists yet, so v1 ships a placeholder
  name ("Stat Race") and a starting look Claude designed: dark navy
  background, bright saturated bars, white Inter text. The user can change
  all of it later in `channel.toml`.
- **Encoding:** raw RGBA frames piped to `ffmpeg` → H.264 (CRF 18, yuv420p,
  60 fps). Audio: music trimmed to video length, 2 s fade-in, 3 s fade-out,
  muxed in the same ffmpeg call.
- **Thumbnail:** `barrace thumb` renders a chosen moment (default: final
  frame) at 1280×720 with an oversized title overlay.
- **CLI:** `barrace validate|render|thumb <week-dir> [--format main|short|both] [--preview]`
  (`--preview` = 30 fps, 720p, for quick checks).
- Fonts: one bundled open font (e.g. Inter, OFL) in `assets/fonts/`.

## Research agent (Python, `research/`)

- Input: topic string + optional year range. Uses Claude API with web search
  to find authoritative sources (World Bank, OWID, Wikipedia, Forbes, etc.),
  builds the CSV, marks each value `estimated=true` when no source gives it
  directly, fills `source` URLs, and drafts `video.toml` with `reviewed=false`.
- **Bulk data:** the agent picks sources, then fetches whole datasets in
  code (World Bank API, OWID CSVs, Wikipedia tables) rather than one value
  per search. A dataset-level URL counts as `source` for every value from it.
  LLM extraction is only for small gaps, and those values are marked
  `estimated=true`. Only items that ever reach the top N are kept.
- Downloads flags/logos into `images/` (flags from an open flag set; logos
  from Wikimedia Commons); misses are left blank for the initials fallback.
- Writes `research_notes.md`: sources used, what was estimated and how.

## Publish (Python, `publish/`)

- Generates title, description (incl. sources list from CSV), tags via LLM.
- Uploads main video + thumbnail and the Short via YouTube Data API v3
  (one-time OAuth setup, token stored locally, outside git).
- Google locks videos uploaded through an **unaudited** API project to
  private. So v1 uploads as **private**, and the user watches each video
  and clicks publish (one click). In parallel, the user applies for the
  YouTube API audit. Once it passes, uploads switch to **scheduled**
  (configurable day/time).

## Weekly flow

```
barrace-week new "largest economies 1960-2026"   # research agent → week dir
# review data.csv in a spreadsheet, set reviewed = true
barrace render weeks/<dir> --format both
barrace thumb weeks/<dir>
barrace-week publish weeks/<dir>
```

## Build order

1. Renderer core: CSV → silent 16:9 MP4 (validate, interpolation, bars, labels, year counter).
2. Polish: images, axis, intro/end cards, music mux.
3. Portrait layout + Short pacing; thumbnail.
4. Research agent.
5. Publish (metadata + YouTube upload).

Each step is usable on its own; step 1–3 with a hand-made CSV already
produces weekly videos.

## Decisions made by Claude (user may override)

- Project at `~/barrace`; per-week folders under `weeks/`.
- Short = same data at faster pace, ~58 s, top 8 bars.
- Uploads scheduled, not instantly public.
- `reviewed` flag gate in `video.toml`.
- Music from a local royalty-free folder the user fills (e.g. YouTube Audio Library downloads).
- Claude API for the research agent; Python 3.12 via uv.

## Left to the builder

Exact crate choices beyond tiny-skia/cosmic-text, easing curves, palette,
font sizes, colour scheme, LLM prompt wording, OAuth flow details.
