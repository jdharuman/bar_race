# barrace

Turns a CSV of numbers over time into a bar-chart-race video: a 16:9 main
video, a 9:16 Short and a thumbnail, with music, fact captions, the leader's
flag, and a Like/Subscribe end card.

Needs Rust and `ffmpeg` on PATH.

```bash
cargo install --path .
```

## Daily automation

`topics.csv` → Claude researches the top topic → barrace renders → private
YouTube upload → Telegram message → you review and publish. Runs daily from
n8n. See [docs/automation.md](docs/automation.md); to install it on a machine,
ask Claude Code in this repo to "set up barrace automation"
(`.claude/skills/barrace-setup/`).

## Manual flow

```bash
mkdir -p weeks/2026-11-30-my-topic        # add data.csv + video.toml (+ events.csv)
barrace validate weeks/2026-11-30-my-topic
# review data.csv, then set reviewed = true in video.toml
barrace render weeks/2026-11-30-my-topic --format both   # out/main.mp4, out/short.mp4
barrace thumb  weeks/2026-11-30-my-topic                 # out/thumb.png
```

Run commands from the repo root: `channel.toml` and `music/` are read
from the current directory.

`weeks/2026-11-23-sample/` is a complete example (World Bank population,
1990–2023, with captions and a pace section).

### Render options

| flag | meaning |
|---|---|
| `--format main\|short\|both` | which video(s); default `main` |
| `--preview` | half size, 30 fps, for quick checks |
| `--bars N` | bars in the main video (1–20) |
| `--short-bars N` | bars in the Short (1–20, default 12) |
| `--music PATH` | use this track |
| `--no-music` | render silent |

## data.csv

One row per item per time step:

```csv
name,category,image,year,value,source,estimated
India,South Asia,images/in.png,1990,870452165,https://data.worldbank.org/...,false
```

| column | meaning |
|---|---|
| `name` | bar label; unique per item |
| `category` | fallback bar colour group |
| `image` | optional flag/logo, path relative to the week folder (png/jpg); empty → initials |
| `year` | `1999`, `1999-02` (monthly) or `1999-Q2` (quarterly) |
| `value` | number, in raw units (dollars, people…) so every digit spins |
| `source` | URL (the end card lists up to 5 domains) |
| `estimated` | `true` → value gets a `*`, end card shows the estimated share |

The counter shows `1999`, `Feb 1999` or `Q2 1999`, using the finest format
in the file. Gaps are interpolated. Bars grow in over one step before an
item's first point and shrink out over one step after its last.

## video.toml

```toml
title = "Most Populous Countries"
subtitle = "Population (World Bank)"
unit_prefix = ""            # e.g. "$"
unit_suffix = ""
top_n = 10                  # bars in the main video
top_n_short = 12            # bars in the Short
seconds_per_year = 2.0      # default: 2 (yearly), 6 (quarterly), 12 (monthly data)
short_seconds = 58          # Short length, 8–180
number_style = "full"       # "full" = 1,234,567,890 · "compact" = 1.2B
bar_colors = "image"        # "image" = colour from each flag/logo · "category"
music = "song.mp3"          # optional, relative to the week folder
reviewed = false            # render refuses until true

[[pace]]                    # optional: play an era at its own speed
from = 1600
to = 1800
seconds_per_year = 0.45

[colors]                    # optional category → colour (beats image colours)
"South Asia" = "#38bdf8"
```

Pace is always seconds per calendar year. The Short squeezes the same pacing
into `short_seconds`, and starts at a later year if that would be too fast to
read.

## events.csv (optional fact captions)

```csv
from,to,headline,big,caption
1913,1925,US produces,90%,World's Oil
```

While the counter is in `[from, to)`, the caption shows above the leader's
flag. When events overlap, the one with the later `from` wins.

## Music

`music/` ships with four CC0 (public domain) tracks; see `music/CREDITS.md`.
Without `music =` in `video.toml` or `--music`, each video picks one by its
title (same title → same track). The track loops or trims to fit, with a 2 s
fade-in and a 3 s fade-out. Add your own tracks to the same folder.

## Channel

`channel.toml`:

```toml
name = ""          # watermark + end-card name; empty = none
theme = "light"    # "light" (white) or "dark" (navy)
# background = "#ffffff"   # optional overrides
# text = "#1f2937"
```

## Pillar walk (3D style)

A second video style: the camera glides past a row of 3D pillars, one per row,
each with a photo or flag, a name plate and a label (e.g. the year or season).
It suits "record holder over time" topics. Set it in `video.toml`:

```toml
title = "Tallest Building in the World"
style = "pillars"
unit_suffix = " m"
seconds_per_pillar = 3.0    # default 2.5
pillar_color = "#d9822b"    # one colour for every pillar (default)
pillar_colors = "single"    # or "image": colour from each image
reviewed = true
```

Data goes in `pillars.csv` (instead of `data.csv`), rows in display order:

```csv
label,name,value,detail,image,source,estimated
2010,Burj Khalifa,828,"Dubai, UAE",images/ae.png,https://en.wikipedia.org/...,false
```

`label`, `name` and `value` are required. `render`, `thumb` and `validate`
work the same; the Short shows the last rows that fit at ≥ 0.8 s each.
Race-only settings (`top_n`, `[[pace]]`, `--bars`, `thumb --year`) are
ignored. The walk's sky look is fixed; the intro and end cards follow
`channel.toml`.

**Photos:** the renderer only uses images in the folder. Use images you have
the rights to. The daily research step downloads only public-domain flags and
free-licence Wikimedia Commons files (CC0, public domain, CC BY, CC BY-SA) and
records each licence. Photos of people are often copyrighted and a common
cause of takedowns. A row without an image gets an initials tile.

Example: `weeks/2026-11-30-tallest-buildings/`.

## Quiz Short (style = "quiz")

A 9:16 flag / capital quiz, made from `quiz.csv` (`image,prompt,a,b,c,d,answer,fact`,
answer = 1–4) by `automation/quiz.py`; see `docs/automation.md`. Render with
`--format short` (there is no main video or thumbnail).

## Line chart (style = "lines")

A third style: a few series (1–6) drawn as lines growing left to right, a
circle and `Name  value` label at each line's head, the y-axis rising as the
values grow. The daily `stocks` topics use it for "value of $1,000 invested"
comparisons. Data is a normal `data.csv` (one `name` per line, monthly
`year` values work best). In `video.toml`:

```toml
title = "Apple vs Microsoft vs Nvidia"
subtitle = "Value of $1,000 invested in {start}, dividends reinvested (Yahoo Finance)"
style = "lines"
rebase = 1000          # optional: every line starts at 1,000 at the first shown month
unit_prefix = "$"
seconds_per_year = 6
reviewed = true
```

- `rebase`: each line is drawn as `rebase × value / value at the start`, so
  `data.csv` keeps the raw (adjusted) prices and stays checkable. With
  `rebase`, every name must have a value > 0 at the first month.
- `{start}` in `title`/`subtitle` becomes the first month the video shows.
  The Short may start later than the data (it trims long histories); it is
  rebased there too, so its subtitle stays true.
- Line colours come from a fixed high-contrast palette (override per name
  or category with `[colors]`). Images, if given, appear in the head circles.
- `events.csv` captions appear top-left of the chart (below it in the Short).
- `look = "neon"`: black background, vivid glowing lines with a colour haze
  beneath, black intro/end cards (overrides `channel.toml` colours for this
  video). Default: the channel theme.
- Bar-race settings (`top_n`, `--bars`, `thumb --year`) are ignored.
