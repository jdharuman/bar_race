# barrace

Turns a CSV of yearly numbers into a bar-chart-race video: a 16:9 main
video, a 9:16 Short and a thumbnail, with music.

Needs Rust and `ffmpeg` on PATH.

```bash
cargo install --path .
```

## Weekly flow

```bash
mkdir -p weeks/2026-11-30-my-topic        # add data.csv + video.toml (see below)
barrace validate weeks/2026-11-30-my-topic
# review data.csv, then set reviewed = true in video.toml
barrace render weeks/2026-11-30-my-topic --format both   # out/main.mp4, out/short.mp4
barrace thumb  weeks/2026-11-30-my-topic                 # out/thumb.png
```

Run commands from the repo root: `channel.toml` and `music/` are read
from the current directory. `--preview` renders at half size and 30 fps
for quick checks.

`weeks/2026-11-23-sample/` is a complete example (World Bank population,
1990–2023).

## data.csv

One row per item per year:

```csv
name,category,image,year,value,source,estimated
India,South Asia,images/in.png,1990,870.45,https://data.worldbank.org/...,false
```

| column | meaning |
|---|---|
| `name` | bar label; unique per item |
| `category` | sets the bar colour |
| `image` | optional path relative to the week folder (png/jpg); empty → initials |
| `year`, `value` | integer year, number |
| `source` | URL (the end card lists up to 5 domains) |
| `estimated` | `true` → value gets a `*`, end card shows the estimated share |

Missing years are interpolated. Bars grow in over the year before an item's
first point and shrink out over the year after its last.

## video.toml

```toml
title = "Most Populous Countries"
subtitle = "Population, millions (World Bank)"
unit_prefix = ""        # e.g. "$"
unit_suffix = "M"
top_n = 10              # bars in the main video
top_n_short = 8         # bars in the Short
seconds_per_year = 3.5  # main video pacing
short_seconds = 58      # Short length
music = "song.mp3"      # optional, relative to the week folder
reviewed = false        # render refuses until true
[colors]                # optional category → colour
"South Asia" = "#38bdf8"
```

## Music

Drop royalty-free tracks (mp3/m4a/wav/ogg/flac) into `music/`, for example
from the YouTube Audio Library. Without `music =` in `video.toml`, each video
picks a track by its title (same title → same track). The track loops or
trims to fit, with a 2 s fade-in and a 3 s fade-out. If there's no music,
the video renders silent with a warning.

## Channel

`channel.toml` holds the channel name (watermark, end card) and colours.
