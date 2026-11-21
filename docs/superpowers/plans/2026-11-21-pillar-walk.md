# 3D Pillar Walk Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A second video style, `style = "pillars"`: a camera glides past a row of 3D pillars (one per row of `pillars.csv`) with photos, name plates, sky, skyline and a glossy floor, rendered by the existing pipeline.

**Architecture:** New modules `pillars.rs` (data), `camera.rs` (perspective projection, camera path, height-scale follow) and `walk.rs` (scene painter). `timeline.rs` gains `PillarTimeline`. `render.rs` branches on `cfg.style` and reuses `Encoder`, music picking, and the intro/end cards from `draw::Painter` through a new `Painter::paint_card`.

**Tech Stack:** unchanged (tiny-skia, cosmic-text, image, csv, serde, toml, clap, anyhow, ffmpeg).

**Spec:** `docs/superpowers/specs/2026-11-21-pillar-walk-design.md`

## Global Constraints

- `video.toml`: `style = "race" | "pillars"` (default `race`), `seconds_per_pillar` (default 2.5, range (0, 30]), `pillar_color` (default `#d9822b`), `pillar_colors = "single" | "image"` (default `single`).
- `pillars.csv` columns: required `label,name,value`; optional `detail,image,source,estimated`. File order is kept. `value` ≥ 0.
- Main timeline: intro 3 s, walk = rows × `seconds_per_pillar`, hold 2 s, end card 5 s. Short: intro 1.5 s, hold 1.0 s, end card 2.5 s, walk = `short_seconds` − 5; if that gives < 0.8 s per pillar, keep only the last `floor(walk / 0.8)` rows.
- Values on plates use `number_style`/`unit_prefix`/`unit_suffix`.
- A pillar is never drawn shorter than its plate plus a margin.
- Intro and end cards use the channel theme (existing `Painter` cards); the walk uses its own sky palette: sky `#4f8fdc → #cfe3f7`, skyline `#3b4a63`, floor `#1e2a3a`, plate `#111827`, value gold `#facc15`.
- Portrait (Short): about 3 pillars visible instead of about 5; bottom 12% kept clear.
- Race-only settings (`top_n`, `--bars`, `[[pace]]`, `thumb --year`) are ignored in pillars mode; `validate` warns if `top_n`/`[[pace]]` are set.
- The tool never downloads photos.
- **Height follow (plan decision, refines the spec's "camera height and distance follow"):** pillar widths stay constant on screen. A smoothed *vertical* scale makes the local maximum (pillars at index p−1.5 … p+2.5) reach a target height. This keeps the focused pillar framed without zooming widths, which matters for 5000× value ranges.

## Review Focus

1. **One-row `pillars.csv`:** renders with a static camera, no panic or NaN. Tested in Tasks 3 and 6.
2. **Zero values** (all zero, or one zero among large ones): the scale never becomes NaN or inf; zero pillars get the minimum height. Tested in Tasks 2 and 5.
3. **Huge range between neighbours** (8,000 next to 40,000,000): the small pillar is still drawn at minimum height with a readable plate. Tested in Task 5.
4. **Very long name or detail:** plate text shrinks to the face width and stays inside the front face. Tested in Task 5.
5. **Missing or unreadable image:** an initials tile and a warning, no crash. Tested in Task 5.

---

### Task 1: Config and pillars data

**Files:** Create `src/pillars.rs`; Modify `src/config.rs`, `src/validate.rs`, `src/main.rs`

**Interfaces:**
- Produces:
  - `config::Style { Race, Pillars }`, `config::PillarColors { Single, Image }` (lowercase serde, defaults Race/Single); `VideoConfig { style, seconds_per_pillar: f64, pillar_color: String, pillar_colors, .. }`; `problems()` checks `seconds_per_pillar` in (0, 30] and that `pillar_color` is `#rrggbb`.
  - `pillars::Pillar { label: String, name: String, value: f64, detail: String, image: Option<PathBuf>, source: String, estimated: bool }`
  - `pillars::parse(week_dir: &Path) -> Result<(Vec<Pillar>, Vec<String>)>` (errors carry line numbers); `pillars::load(week_dir) -> Result<Vec<Pillar>>` (bails on errors)
  - `validate::run` branches on `style`. Pillars mode: `items` = rows, `first`/`last` = 0, granularity Year, and warnings for `top_n != 10` or a non-empty `pace`. The CLI prints `pillars: N` instead of `time:` when the style is pillars.

- [ ] **Step 1: Write failing tests:**

```rust
// pillars.rs
#[test] fn parses_rows_in_order() { /* 3 rows "1946-47,Joe Fulks,8000,Philadelphia Warriors,,," … → names in file order, value 8000.0, detail set */ }
#[test] fn optional_columns_may_be_missing() { /* header "label,name,value" only → ok, image None, estimated false */ }
#[test] fn bad_rows_have_line_numbers() { /* value "-1" line 2, value "abc" line 3, empty name line 4 → errors mention line 2, 3, 4 */ }
#[test] fn missing_image_is_error() { /* image=images/nope.jpg → error contains "nope.jpg" */ }
// config.rs
#[test] fn style_defaults_and_parse() { /* default Race; toml "style = \"pillars\"" → Pillars; seconds_per_pillar default 2.5; pillar_color "#d9822b" */ }
#[test] fn bad_pillar_settings() { /* seconds_per_pillar 0 and pillar_color "orange" → both in problems() */ }
// validate.rs
#[test] fn validate_pillars_week() { /* temp week with pillars.csv (2 rows) and style=pillars, top_n=12 → items 2, no errors, a warning mentioning "top_n" */ }
```
- [ ] **Step 2:** Run `cargo test`. Expected: FAIL (compile).
- [ ] **Step 3: Implement.** Use the `csv` crate with header lookup, as in `data.rs`. Image paths resolve against the week dir. `estimated` accepts true/false/1/0/yes/no.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: pillars.csv data and pillar settings`.

---

### Task 2: Camera, path and height follow

**Files:** Create `src/camera.rs`

**Interfaces:**
- Produces:
  - `camera::SPACING: f32 = 1.6`, `PILLAR_W: f32 = 1.0`, `PILLAR_D: f32 = 0.7` (world units; pillar i spans x ∈ [i·SPACING − 0.5, i·SPACING + 0.5], z ∈ [0, PILLAR_D], y ∈ [0, h])
  - `camera::Camera { x: f32, eye_y: f32, dist: f32, focal: f32, cx: f32, horizon: f32 }`; `Camera::for_frame(width: u32, height: u32, portrait: bool) -> Camera` (all constants live here); `project(&self, x: f32, y: f32, z: f32) -> Option<(f32, f32)>` (None if z + dist ≤ 0.05)
  - `camera::focus_x(p: f64, portrait: bool) -> f32`: the camera x for focus position p (pillar index, fractional), placing pillar p a little left of centre
  - `camera::ease_walk(u: f64) -> f64`: maps 0..1 → 0..1 with gentle ease at both ends (monotone, ease(0)=0, ease(1)=1)
  - `camera::HeightFollow::new() -> HeightFollow`; `step(&mut self, values: &[f64], p: f64, dt: f64) -> f32`: the vertical scale (world height per value unit) so the max value in indices [p−1.5, p+2.5] maps to `TARGET_H` (= 2.4). Exponential smoothing with τ = 0.6 s; the first call snaps. Returns 1.0 when the local max is 0.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn projects_axis_point_to_centre() { /* cam.x=5: project(5, eye_y, 0) == (cx, horizon) ±0.01 */ }
#[test] fn farther_is_smaller() { /* width of [x, x+1] at z=0 > width at z=3 */ }
#[test] fn behind_camera_none() { /* project(.., .., -dist - 1) == None */ }
#[test] fn ease_walk_monotone() { /* ease(0)=0, ease(1)=1, strictly increasing over 100 samples, ease(0.5)=0.5 ±0.01 */ }
#[test] fn follow_targets_local_max() { /* values [1,1,1,100,1]: at p=0 scale*1 ≈ TARGET_H (max over 0..2 is 1); at p=2 after 3 s scale*100 ≈ TARGET_H ±2% */ }
#[test] fn follow_handles_zero_and_nan_free() { /* values all 0 → 1.0; values [0, 5e7] → finite positive */ }
#[test] fn portrait_shows_fewer_pillars() { /* pillars whose centre projects inside the frame at p=10: landscape ≥ 4, portrait ≤ 3 */ }
```
- [ ] **Step 2:** Run `cargo test camera`. Expected: FAIL.
- [ ] **Step 3: Implement.** Pinhole model: `sx = cx + focal·(x − cam.x)/(z + dist)`, `sy = horizon − focal·(y − eye_y)/(z + dist)`. Starting constants (tune by eye in Task 4): landscape `dist = 5.5`, `eye_y = 1.1`, `focal` such that a TARGET_H pillar at z=0 spans about 0.62·height, `horizon ≈ 0.6·height`. In portrait, `focal` is scaled so about 3 pillars fit the width. `focus_x(p) = p·SPACING + 0.9` (landscape) or `+ 0.4` (portrait).
- [ ] **Step 4:** Run `cargo test camera`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: camera projection, walk easing and height follow`.

---

### Task 3: Pillar timeline

**Files:** Modify `src/timeline.rs`

**Interfaces:**
- Consumes: `config::VideoConfig.seconds_per_pillar`, `short_seconds`
- Produces: `timeline::PillarTimeline { fps, first: usize /*first row shown*/, n: usize /*rows shown*/, intro, walk, hold, outro: f64 }`; `PillarTimeline::new(fmt: Format, cfg: &VideoConfig, rows: usize, fps: u32)`; `total_seconds`, `frame_count`; `phase_at(frame) -> Phase`, where `Phase::Race { t }` carries the focus position p in absolute row index (from `first` to `first + n − 1`), with `camera::ease_walk` applied.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn pillar_main_duration() { /* 60 rows, default 2.5 → walk 150, total 3+150+2+5 = 160 */ }
#[test] fn pillar_short_scales() { /* 20 rows, short 58 → walk 53, 53/20 = 2.65 ≥ 0.8 → first 0, n 20 */ }
#[test] fn pillar_short_keeps_last_rows() { /* 100 rows, short 58 → 53/100 < 0.8 → n = floor(53/0.8) = 66, first = 34 */ }
#[test] fn pillar_phases() { /* first race frame p == first as f64; last race frame p ≈ first+n-1 (±0.01); then Hold; last frame End{t>0.98} */ }
#[test] fn single_row_is_static() { /* 1 row → every Race frame has p == 0.0; walk == seconds_per_pillar */ }
```
- [ ] **Step 2:** Run `cargo test timeline`. Expected: FAIL.
- [ ] **Step 3: Implement.** Walk position: `p = first + ease_walk(s / walk) · (n − 1)`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: pillar walk timeline`.

---

### Task 4: Scene painting (background, pillars, reflections)

**Files:** Create `src/walk.rs`; Modify `src/main.rs`

**Interfaces:**
- Consumes: `camera::*`, `pillars::Pillar`, `config::{VideoConfig, PillarColors}`, `draw::{hex, dominant_color}`
- Produces: `walk::WalkPainter::new(pillars: &[Pillar], cfg: &VideoConfig, width: u32, height: u32, portrait: bool) -> Result<WalkPainter>`; `paint(&mut self, pm: &mut Pixmap, p: f64, scale: f32)` (scale from `HeightFollow`); `pub last_strings: Vec<String>`; `pillar_rect(&self, i: usize, p: f64, scale: f32) -> Option<(f32, f32, f32, f32)>` (the front face on screen, for tests); `pillar_color(&self, i: usize) -> Color`.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn sky_at_top_floor_at_bottom() { /* pixel (w/2, 2) ≈ sky top colour; pixel (w/2, h-2) ≈ floor colour (±25) */ }
#[test] fn focused_pillar_front_face_coloured() { /* 5 pillars, p=2: centre of pillar_rect(2) ≈ pillar_color(2) */ }
#[test] fn side_face_darker_than_front() { /* pillar right of camera: pixel just left of its front rect (side face) has lower luminance than the front */ }
#[test] fn reflection_below_floor_line() { /* a pixel below pillar 2's base is tinted toward the pillar colour vs plain floor (differs by > 10 in red) */ }
#[test] fn image_pillar_colour_mode() { /* pillar_colors Image + solid green PNG → pillar_color == green; Single → #d9822b */ }
```
- [ ] **Step 2:** Run `cargo test walk`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - **Sky:** a tiny-skia `LinearGradient` down to the horizon.
  - **Clouds:** soft ellipses from a fixed-seed LCG, at parallax 0.05 of the camera x.
  - **Skyline:** seeded building rectangles with a few lit-window dots, standing on the horizon, at parallax 0.25.
  - **Floor:** a gradient below the horizon.
  - **Pillars:** draw the visible ones (projected x inside the frame ± one pillar) farthest-from-camera first. Front face = the projected z=0 rectangle. The side face is the quad toward the camera (left face for pillars right of the camera, right face otherwise), with the colour ×0.7. Top = the quad at y = h with the colour ×1.15 (clamped).
  - **Reflections:** mirror y → −y, drawn before the pillars at 22% alpha, fading to 0 over 0.6 of the pillar height. Heights come from `max(value · scale, MIN_H)`; MIN_H is defined in Task 5 (use 0.35 here).
  - **Visual check:** render a throwaway frame to PNG and tune the camera constants until a 5-pillar landscape and a 3-pillar portrait look right.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: pillar walk scene painting`.

---

### Task 5: Plates, photos, title

**Files:** Modify `src/walk.rs`

**Interfaces:**
- Produces: `walk::MIN_PLATE_FRACTION`: a pillar's world height is `max(value · scale, plate_h · 1.15)`, where `plate_h = 0.55 · PILLAR_W` (world). Plate drawn inside the front face from its top: dark panel `#111827` with `name` (white, bold), value (gold, via `format_value(.., cfg.number_style)`), `detail` (white 80%), `label` (bold, larger, below the panel, in the plate text colour). The photo billboard (4:5 centre crop, width = `PILLAR_W`) stands on the pillar top and is projected at z=0. With no image or a failed load: an initials tile in the pillar colour ×0.6. Video title is small at top-left (`cfg.title`).

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn plate_strings_for_visible_pillars() { /* p=1 → last_strings contains "Joe Fulks", "8,000" (Full style, no prefix), "Philadelphia Warriors", "1946-47" */ }
#[test] fn tiny_pillar_keeps_plate_height() { /* values [8000, 40_000_000], p=1 after settle: pillar_rect(0).h ≥ plate height on screen (≥ 0.55·front width) */ }
#[test] fn long_name_fits_face() { /* name 120 chars: no plate text pixels outside pillar_rect(i) ±2 px (diff vs short-name render, as in long_caption_fits_box) */ }
#[test] fn missing_image_initials() { /* unreadable image file (garbage bytes) → no panic; last_strings contains initials "JF" */ }
#[test] fn photo_above_pillar() { /* red PNG photo: pixel at (rect centre x, rect.y − 0.3·rect.w) ≈ red */ }
```
- [ ] **Step 2:** Run `cargo test walk`. Expected: FAIL.
- [ ] **Step 3: Implement.** Load photos once at 400×500 (`resize_to_fill`, Lanczos3) as premultiplied Pixmaps. Draw them with `draw_pixmap` and a scale `Transform` to the projected size (bilinear `FilterQuality`). Plate text sizes scale with the front-face width; fit each line with a `fit_size` equivalent (≥ 4 px floor). The `Text` instance is owned by `WalkPainter`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Look at a rendered frame by eye.
- [ ] **Step 5:** Commit with the message `feat: pillar plates, photos and title`.

---

### Task 6: Render, Short, thumbnail, cards

**Files:** Modify `src/render.rs`, `src/draw.rs`, `src/main.rs`; Test `tests/cli.rs`; Create `tests/fixtures/pillars/pillars.csv`, `tests/fixtures/pillars/video.toml`

**Interfaces:**
- Consumes: Tasks 1–5, `Painter`, `Encoder`, `pick_music`/`RenderOpts`
- Produces:
  - `draw::Painter::paint_card(&mut self, pm: &mut Pixmap, phase: &Phase)`: draws only the intro or end-card overlay onto an existing frame (no clear). It is a no-op for Race/Hold.
  - `pillars::to_dataset(rows: &[Pillar]) -> Dataset`: each row becomes an `Item` with one point, so `Painter` cards get sources and estimated share.
  - `render::render` and `render::thumb` branch on `cfg.style == Style::Pillars` into `render_pillars(week_dir, fmt, opts)` and `thumb_pillars(week_dir)`. Output names are unchanged (`main.mp4`, `short.mp4`, `thumb.png`).

- [ ] **Step 1: Write failing tests** (cli.rs; fixture of 4 rows, `seconds_per_pillar = 0.5`, `reviewed = true`, `style = "pillars"`):

```rust
#[test] fn render_pillars_preview() { /* main: 960x540, duration 3 + 4*0.5 + 2 + 5 = 12.0 ±0.1 */ }
#[test] fn render_pillars_short() { /* --format short, short_seconds 10 → 540x960, duration 10.0 ±0.1 */ }
#[test] fn pillars_thumb() { /* thumb → 1280x720 png */ }
#[test] fn pillars_single_row_renders() { /* 1-row CSV → exit 0 */ }
#[test] fn pillars_refuses_unreviewed() { /* reviewed=false → exit != 0, "reviewed" in stderr */ }
```
Unit test in draw.rs: `paint_card_leaves_race_untouched`. `paint_card(Race)` must not change a filled pixmap, while `paint_card(End{t:0.5})` must add the "SUBSCRIBE" string.

- [ ] **Step 2:** Run `cargo test`. Expected: FAIL.
- [ ] **Step 3: Implement.** In `render_pillars`, for each frame: get the phase → p (Intro → first row, Race → p, Hold/End → last row) → `HeightFollow::step(values, p, dt)` → `WalkPainter::paint` → `Painter::paint_card(phase)` → encode. Music resolution and the `reviewed` gate are as in the race. The thumbnail shows the last row settled for 2 s, plus the existing big-title overlay style (reuse `Painter` title drawing via a small `paint_thumb_title(pm)` helper extracted from `paint_thumb`).
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Render the fixture preview and look at frames from the intro, mid-walk, the Short and the end card.
- [ ] **Step 5:** Commit with the message `feat: render pillar walk videos and thumbnails`.

---

### Task 7: Sample pillar week and README

**Files:** Create `weeks/2026-11-30-tallest-buildings/pillars.csv`, `video.toml`, `images/*.png`; Modify `README.md`

- [ ] **Step 1: Sample data:** "Tallest Building in the World" record holders (height in metres, `detail` = city, image = country flag PNG copied from the population sample's flags, source = Wikipedia "History of the world's tallest buildings"): 1901 Philadelphia City Hall 167 m; 1908 Singer Building 187; 1909 Met Life Tower 213; 1913 Woolworth Building 241; 1930 40 Wall Street 283; 1930 Chrysler Building 319; 1931 Empire State Building 381; 1972 World Trade Center 417; 1974 Sears Tower 442; 1998 Petronas Towers 452; 2004 Taipei 101 508; 2010 Burj Khalifa 828. `unit_suffix = " m"`, `seconds_per_pillar = 3.0`, `reviewed = true`. Add the flags needed for MY, TW, AE (copy from flagcdn as in the population sample, one-off).
- [ ] **Step 2: README:** a "Pillar walk" section covering `style`, the `pillars.csv` columns, `seconds_per_pillar`, `pillar_color`/`pillar_colors`, the photo-rights note (free-licence images such as Wikimedia Commons; the tool never downloads photos), and the race-only settings that are ignored.
- [ ] **Step 3: Verify:** run `cargo test` (all green), then `time ./target/release/barrace render weeks/2026-11-30-tallest-buildings --format both` and `thumb`. Look at frames. Expected: renders in about the same time per video second as the race; record the time in the commit.
- [ ] **Step 4:** Commit with the message `docs: tallest-buildings pillar sample, README`.
