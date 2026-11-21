# barrace v2 — faster-feeling, richer videos

## Intent

The v1 videos feel slow and plain next to channels the user wants to compete
with (Global Stats "Richest Tech Billionaires 1996–2026", Gozhda "Largest
Economies 1600–2024"). Goal: a viewer stays because the screen is always
moving and has something to read, and is prompted to like and subscribe at
the end. Everything below is approved by the user; it extends the existing
Rust renderer (spec `2026-11-20-barrace-design.md`).

## What the reference videos do (measured)

- **Global Stats:** monthly data ("Feb 1999"), ~2 s per month step, 15–16
  dense bars, story panel (era heading + one-line event + photo).
- **Gozhda:** full-digit values (`$77,246,849,166`) whose low digits spin
  every frame; pace varies by era: ~0.45 s/year for 1625–1800, ~1.7 s/year
  for 1800–2024; big fact captions ("US produces / 90% / World's Oil");
  white background; bars coloured after the flag; flag at bar tip.

## Goals (all default-on)

1. **Sub-year time.** The CSV `year` column accepts `1999`, `1999-02`
   (monthly) or `1999-Q2` (quarterly). Internally time is fractional years
   (`1999 + (month-1)/12`, `1999 + (q-1)/4`). The counter shows `1999`,
   `Feb 1999` or `Q2 1999`, using the finest granularity in the data.
2. **Pace by era.** Default 2.0 s per year for yearly data (was 3.5); see Decisions for sub-year defaults. Optional
   sections in `video.toml`:
   ```toml
   [[pace]]
   from = 1600
   to = 1800
   seconds_per_year = 0.45
   ```
   Years not covered by a section use `seconds_per_year`. The timeline maps
   video time → data time piecewise-linearly.
3. **Full-digit numbers.** `number_style = "full"` (default) shows values
   ≥ 1000 as whole numbers with thousands separators and no rounding to
   K/M/B; below 1000, one decimal. `"compact"` keeps the v1 behaviour. Axis
   tick labels use the same style. Data should be in raw units (e.g. dollars,
   not billions) for the spinning-digit effect.
4. **Fact captions.** Optional `events.csv` in the week folder:
   ```csv
   from,to,headline,big,caption
   1913,1925,US produces,90%,World's Oil
   ```
   `from`/`to` use the same time formats as `data.csv`. While data time is in
   `[from, to)`, the caption shows above the year counter: `headline` (accent
   red, bold), `big` (very large), `caption` (medium). Fades in and out over
   0.3 s. Overlapping events: the later `from` wins. `barrace validate` checks
   the file.
5. **Leader badge.** The rank-1 item's image (or initials circle) is drawn
   large above the year counter (below any caption). On a leader change, the
   old badge fades out and the new one fades in over 0.4 s.
6. **Bar colour from image.** `bar_colors = "image"` (default): each item's
   bar colour is the dominant colour of its image (most frequent colour after
   quantising, ignoring near-white, near-black and transparent pixels), and
   falls back to the category palette when there's no image or no dominant
   colour. `"category"` = v1 behaviour. `[colors]` overrides still win.
7. **Light theme.** `theme = "light"` (default, in `channel.toml`):
   white background, near-black text, grey gridlines, dark-grey year counter.
   `theme = "dark"` = v1 look. `background`/`text` keys still override.
8. **Like + Subscribe end card.** Last 5 s of the main video, last 2.5 s of
   the Short. A Like button (thumbs-up + "LIKE") and a red "SUBSCRIBE" button
   with a bell pop in (scale ease-out). A drawn cursor clicks Like (it fills
   blue), then clicks Subscribe (it turns grey and reads "SUBSCRIBED"), then
   the bell swings ±20° twice. All drawn with vector paths; no YouTube logo
   or other trademarked artwork. The main video keeps the sources and
   estimated-share lines above the buttons; the Short shows only the buttons.
9. **No "Stat Race".** `channel.toml` `name` defaults to empty. When empty,
   no watermark and no channel line on the end card.
10. **More bars in Shorts.** `top_n_short` default 12. CLI overrides:
    `barrace render <dir> --bars N --short-bars N` (1–20, same range check
    as config).
11. **Music in the repo.** 3–4 tracks under `music/` that are public
    domain / CC0 (no attribution needed, no Content ID risk), each listed in
    `music/CREDITS.md` with source URL and licence. Rendering picks one by
    default (existing hash-by-title rule). Overrides: `music = "…"` in
    `video.toml`, `--music <path>` on the CLI, `--no-music` to render silent.

## Non-goals (this round)

- Story panel with large photos per event (needs per-event photos; do it
  after the research agent exists).
- Logos inside bars, person photos, two images per bar.
- Research agent and YouTube upload (separate plans).
- Voiceover.

## Anti-goals

- Copyrighted music or tracks that need on-screen credit.
- YouTube's logo or other trademarked UI art in the end card.
- Copying either channel's exact layout, fonts or branding. Genre features only.

## Constraints

- Keep the render speed in the same range as v1: a 4-minute main video at
  1080p60 in about 5 minutes on this machine.
- The light theme must stay readable on the Short at phone size.
- v1 week folders keep working: an integer `year` column and no
  `events.csv` or `[[pace]]` sections render without changes.

## Approaches

| | Approach | Trade-off |
|---|---|---|
| **Picked** | Extend the existing renderer modules: data (time parsing), timeline (pace sections), fmt (full style), draw (theme, badge, captions, end card), plus a new `events.rs`. | Small, well-tested base; each feature is a contained change. |
| Rejected | Rewrite the renderer around a scene graph or animation framework. | No need: the features are overlays and data/time changes, not a new rendering model. |
| Rejected | Burn captions and end card in a second ffmpeg pass (drawtext/overlay). | Splits the look across two systems, ffmpeg drawtext styling is limited, and it adds a re-encode pass. |

## Decisions made by Claude (user may override)

- Default pace 2.0 s/year. Gozhda's main era runs ~1.7 s/year and Global
  Stats ~2 s/step, so 2.0 sits between them.
- Light theme is the default, since both reference videos use white.
- Caption position: right side, above the leader badge, which sits above
  the year.
- End card lengths: 5 s main, 2.5 s Short. The Short's race shortens to fit.
- Short timeline becomes intro 1.5 s + race + hold 1.0 s + end 2.5 s, still
  totalling `short_seconds`.
- Music source: CC0/public-domain libraries (e.g. FreePD); the builder
  chooses 3–4 energetic instrumental tracks and records them in CREDITS.md.

- **Pace units:** pace is always seconds per *calendar year*, in both
  `seconds_per_year` and `[[pace]]`. When `seconds_per_year` isn't set, the
  default depends on the data's granularity: yearly 2.0 (2 s per step),
  quarterly 6.0 (1.5 s per quarter), monthly 12.0 (1 s per month). Monthly
  1996–2026 is then about 6 minutes; set 24 for Global Stats' ~2 s per month.
- **Shorts:** pace sections are scaled by one common factor so the race fits
  `short_seconds`. If the resulting average is faster than the readable
  floor (0.8 s per year for yearly data, 3 s per year for sub-year data), the
  start moves later to a whole year, as in v1.
- **Grow-in/shrink-out ramp** lasts one data step: a year for yearly data,
  a quarter or a month for sub-year data.
- **v1 week folders** render without edits. Their look changes to the new
  defaults (light theme, full digits, etc.); nothing keeps the v1 look
  automatically. The sample week gets updated to raw units and the new
  features.
- **Where settings live:** `number_style` and `bar_colors` go in `video.toml`
  (per video). `theme` goes in `channel.toml` (channel-wide).

## Left to the builder

Exact sizes and positions, easing curves, cursor and icon shapes, the
dominant-colour algorithm details, and which specific CC0 tracks to use.
