# barrace

Turns a CSV of numbers over time into a bar-chart-race video: a 16:9 main
video, a 9:16 Short and a thumbnail, with music, fact captions, the leader's
flag, and a Like/Subscribe end card.

Needs Rust and `ffmpeg` on PATH.

```bash
cargo install --path .
```

## Weekly flow

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
