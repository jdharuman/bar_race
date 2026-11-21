# barrace — 3D pillar walk video style

## Intent

Add a second weekly video format next to the bar race: a "record holder over
time" walk. A row of 3D pillars, one per data row (e.g. each season's
highest-paid player), with the camera gliding past them while pillar height
tracks the value. Reference: WorldPedia3D "Highest Paid NBA Players by
Season". It shares the bar race's tool, pipeline and polish (music, end card,
Short, thumbnail). The bar race stays unchanged.

## What the viewer sees

```
  ☁  sky gradient, drifting clouds   ☁
  ▂▃▅▂▆▃▂ skyline silhouette (parallax) ▅▂▃▆▂
            [photo]
 [photo]   ┌──────┐   [photo]
┌──────┐   │ NAME │  ┌──────┐
│ NAME │   │$2.2M │  │ ...  │
│$1.0M │   │ team │  │      │
│1979  │   │1982-3│  │      │
════════ glossy floor with faded reflections ═══
```

- One pillar per CSV row, in file order, evenly spaced left → right.
- The camera slides right at a steady pace, easing in at the start and out at
  the end. The pillar being "visited" sits a little left of centre, so the
  upcoming (usually taller) pillars are visible on the right.
- The camera height and distance follow the size of the pillars around the
  focused one (smoothed), so the focused pillar always fills the frame
  nicely. A tiny pillar next to a huge one looks tiny, which is honest.
- Each pillar has a lit front face, a darker side face (the side facing the
  camera) and a gradient top. A photo stands on top as a billboard (4:5 crop),
  and a front plate shows name (white on a dark panel), value (gold,
  full digits), `detail` (e.g. team) and `label` (e.g. season).
- Background: a sky gradient with soft procedural clouds (slowest parallax),
  a procedural skyline silhouette (medium parallax; our own art, generated
  from a seed), and a glossy floor showing a vertically flipped, faded
  reflection of the pillars.
- The video title stays small at the top-left during the walk. The intro
  title card, Like/Subscribe end card, music and thumbnail work as in the
  bar race.

## Goals

1. `video.toml` gets `style = "race" | "pillars"` (default `race`). The
   `render`, `thumb` and `validate` commands follow it. No new subcommand.
2. Input `pillars.csv` (instead of `data.csv`):
   ```csv
   label,name,value,detail,image,source,estimated
   1982-83,Moses Malone,2200000,Philadelphia 76ers,images/malone.jpg,https://…,false
   ```
   `label` and `name` are required. `value` is a number ≥ 0. `detail`,
   `image`, `source` and `estimated` are optional (same meaning as in the bar
   race). Rows keep file order. Labels needn't be unique.
3. Pacing: `seconds_per_pillar` (default 2.5). Main timeline: intro 3 s, walk
   = rows × pace, hold 2 s on the last pillar, end card 5 s. The Short
   scales the pace to fit `short_seconds`; if that drops below 0.8 s per
   pillar, the Short covers only the last N rows that fit.
4. Pillar colour: one colour for all pillars, `pillar_color` (hex, default
   warm orange `#d9822b`). Optional `pillar_colors = "image"` takes each
   image's dominant colour instead (existing helper), falling back to
   `pillar_color`. The default stays `"single"`, because photos of people give
   skin-tone colours. The bar race's `bar_colors` doesn't apply here. Front,
   side and top shades derive from the pillar's colour.
5. Portrait (Short) layout: camera closer and narrower, so about 3 pillars
   are visible instead of about 5. Text sized for phones; bottom 12% kept
   clear.
6. Thumbnail: the final pillar framed, with the big title (same style as the
   bar race thumbnail).
7. Speed: the same ballpark as the bar race (a 6-minute 1080p60 video in a
   few minutes).

## Non-goals

- Real lighting, soft shadows or ray-traced reflections (Approach A paints
  the shading).
- Camera orbits, rotations or cuts; it only slides with a smooth follow.
- Several pillars per label (grouped seasons), fact captions, leader badge.
- Downloading photos. The tool only uses images the user provides.

## Anti-goals

- Copying WorldPedia3D's look exactly (their skyline image, colours, font).
  Same genre, our own art.
- Encouraging copyrighted photos: the README says to use images you have
  rights to (e.g. free-licence Wikimedia Commons). Missing images fall back
  to an initials tile.

## Approaches

| | Approach | Trade-off |
|---|---|---|
| **A (picked)** | Our tiny-skia renderer with a small perspective camera: boxes projected in true perspective, painted shading, parallax layers, a flipped-and-faded reflection. | Reuses text, images, music, end card, encoder and tests; fast; no new dependencies. Looks like polished motion graphics rather than a game engine. |
| B | Real 3D on the GPU in Rust (`wgpu`, headless). | True lighting and reflections, but about 3× the code (shaders, meshes, text textures), fragile GPU drivers when headless, and frames that are hard to test. |
| C | Blender scripted from Python. | Most realistic, but a ~1 GB install, a second language and toolchain, and 30–90 min renders per video. Could become a premium option later. |

## Architecture (A)

- `pillars.rs`: load and validate `pillars.csv` → `Vec<Pillar>`.
- `camera.rs`: world → screen perspective projection
  (`screen_x = cx + f·(x−camx)/(z+d)`, `screen_y = horizon − f·(y−camy)/(z+d)`),
  plus the camera path (x along the row; height and distance following the
  smoothed local maximum height).
- `walk.rs`: per-frame scene painting (sky, clouds, skyline, floor,
  reflections, pillars back-to-front, photos, plates, title), sharing
  `Text`, `fmt`, the image loading and `dominant_color`, and the end/intro
  card code from `draw.rs`.
- `timeline.rs` gains a pillar-walk timeline (Intro / Walk{pos} / Hold / End).
- `render.rs` branches on `style`.

## Decisions made by Claude (user may override)

- The format is chosen in `video.toml` (`style`), not by a CLI flag, because
  it belongs to the dataset.
- Default pace 2.5 s per pillar (the reference moves about one pillar every
  2–3 s). 60 rows → about 2.5 min.
- Pillar height is linear in value; the camera's follow-zoom keeps it
  readable, and there's no log scale. A log option can come later if needed.
- Values on plates are static (no count-up). They're formatted with the
  existing `number_style` / `unit_prefix` / `unit_suffix` (default full digits,
  `$2,200,000`); the `$2.2M` in the sketch is shorthand only.
- **Short pillars:** a pillar is drawn at least as tall as its plate plus a
  small margin, so the plate always sits on the pillar's front face near the
  top, with the photo standing on the pillar top above it. Only near-zero
  values get stretched to this minimum.
- **Intro and end cards** use the channel theme (light or dark), exactly as
  in the bar race; the sky palette applies only to the walk itself.
- Race-only settings (`top_n`, `--bars`, `[[pace]]`, `thumb --year`) are
  ignored in pillars mode. `validate` warns if they're set.
- Default palette: sky `#4f8fdc → #cfe3f7`, skyline `#3b4a63`, floor
  `#1e2a3a`, pillar `#d9822b`, plate `#111827`, value gold `#facc15`.

## Left to the builder

The exact camera constants (focal length, follow smoothing, the
"focus a little left of centre" offset), the cloud and skyline generator,
plate typography, and how to fade the reflection.
