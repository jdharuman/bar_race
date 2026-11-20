# barrace Renderer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust CLI `barrace` that turns a week folder (`data.csv` + `video.toml`) into a 16:9 bar-chart-race MP4, a 9:16 Short MP4 (both with music) and a thumbnail PNG.

**Architecture:** Pure-logic modules (data loading, validation, interpolation/ranking, timeline) feed a per-frame `FrameState`. A drawing module paints a `FrameState` onto a `tiny_skia::Pixmap` using a `Layout` (landscape or portrait). Raw RGBA frames are piped into an `ffmpeg` child process, which also muxes music.

**Tech Stack:** Rust (edition 2021), `tiny-skia`, `cosmic-text`, `image` (png+jpeg), `csv`, `serde`, `toml`, `clap` (derive), `anyhow`; external `ffmpeg`/`ffprobe` binaries. Font: Inter (OFL), bundled.

**Spec:** `docs/superpowers/specs/2026-11-20-barrace-design.md`

## Global Constraints

- Main video: 1920×1080, 60 fps, H.264 CRF 18, yuv420p. Short: 1080×1920, 60 fps, same codec.
- `--preview`: 30 fps, half resolution (960×540 / 540×960).
- Main timeline: intro 3 s → race (`seconds_per_year` × years) → hold 3 s → end card 4 s.
- Short timeline: intro 1.5 s → race → hold 1.5 s, total = `short_seconds` (default 58); no end card; start year moves later if pace < 0.8 s/year.
- `top_n` default 10, `top_n_short` default 8.
- Music: 2 s fade-in, 3 s fade-out, trimmed or looped to the video length; only from the configured file or the local `music/` folder.
- Render refuses unless `video.toml` has `reviewed = true` (`validate` just reports it).
- Estimated values: value label gets a `*`; the main end card says `* estimated` with the share of estimated values.
- Channel identity comes from `channel.toml` in the current directory; default name `"Stat Race"`, dark navy background `#0f172a`, white text.
- Open-source deps only. No network access at render time.

## Review Focus

1. **Item that enters or drops out mid-series** (e.g. USSR ends 1991, single-year items): the bar grows from 0 over the year before its first point and shrinks to 0 over the year after its last point. It never blinks. Tested in Task 2.
2. **Ties in value:** rank order is stable (value desc, then name asc), so bars don't flicker. Tested in Task 2.
3. **Fewer items than `top_n`:** renders only the items that exist, without a panic. Tested in Task 4.
4. **Non-Latin names** (`São Tomé`, `中国`, `Côte d'Ivoire`): system-font fallback draws them with no panic or tofu-only output. Tested in Task 4.
5. **Value ranges:** `0.5`, `1234.5`, `1234567` and `2.4e12` format readably (decimals only below 100, thousands separators). Negative values are clamped to a zero-length bar but keep their label. Tested in Task 4.

---

## File Structure

```
Cargo.toml
.gitignore                 target/, weeks/*/out/
channel.toml               default channel identity
assets/fonts/Inter-Regular.ttf, Inter-Bold.ttf
music/.gitkeep             user drops royalty-free tracks here
src/main.rs                clap CLI: validate | render | thumb
src/config.rs              VideoConfig (video.toml), ChannelConfig (channel.toml)
src/data.rs                CSV → Dataset
src/validate.rs            checks + summary report
src/scene.rs               interpolation, ranking, RankTracker → FrameState
src/timeline.rs            frame index → Phase, for Main/Short
src/fmt.rs                 number formatting
src/layout.rs              Layout constants (landscape, portrait)
src/text.rs                cosmic-text → tiny-skia text drawing
src/draw.rs                FrameState + Phase → Pixmap
src/encode.rs              ffmpeg child process + audio args
src/render.rs              orchestration: render(), thumb()
tests/fixtures/week/       data.csv, video.toml (tiny 4-item, 2000–2003)
tests/cli.rs               end-to-end tests (needs ffmpeg/ffprobe)
```

---

### Task 1: Scaffold, config, data loading, `validate`

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `channel.toml`, `music/.gitkeep`, `src/main.rs`, `src/config.rs`, `src/data.rs`, `src/validate.rs`, `tests/fixtures/week/data.csv`, `tests/fixtures/week/video.toml`

**Interfaces:**
- Produces:
  - `config::VideoConfig { title: String, subtitle: String, unit_prefix: String, unit_suffix: String, top_n: usize /*10*/, top_n_short: usize /*8*/, seconds_per_year: f64 /*3.5*/, short_seconds: f64 /*58*/, music: Option<PathBuf>, reviewed: bool /*false*/, colors: BTreeMap<String,String> }` with serde defaults; `VideoConfig::load(path: &Path) -> anyhow::Result<VideoConfig>`
  - `config::ChannelConfig { name: String /*"Stat Race"*/, logo: Option<PathBuf>, background: String /*"#0f172a"*/, text: String /*"#ffffff"*/, accent: String /*"#38bdf8"*/ }`; `ChannelConfig::load_or_default(path: &Path) -> anyhow::Result<ChannelConfig>`
  - `data::Point { year: i32, value: f64, estimated: bool }`
  - `data::Item { name: String, category: String, image: Option<PathBuf> /*resolved against week dir*/, points: Vec<Point> /*sorted by year*/, sources: Vec<String> }`
  - `data::Dataset { items: Vec<Item>, first_year: i32, last_year: i32 }`; `Dataset::load(week_dir: &Path) -> anyhow::Result<Dataset>`; `Dataset::estimated_share(&self) -> f64` (0..1)
  - `validate::Report { errors: Vec<String>, warnings: Vec<String>, items: usize, first_year: i32, last_year: i32, estimated_share: f64, reviewed: bool }`; `validate::run(week_dir: &Path) -> anyhow::Result<Report>`
  - CLI `barrace validate <DIR>`: prints the report; exit code 1 if `errors` is non-empty.

- [ ] **Step 1: Create the crate.** Run `cargo init --name barrace` in the repo root, then add deps: `cargo add tiny-skia cosmic-text csv serde --features serde/derive toml clap --features clap/derive anyhow image --no-default-features --features image/png,image/jpeg`. In `.gitignore`: `target/`, `weeks/*/out/`. In `channel.toml`: the default values above. Fixture `data.csv`: items A, B, C, D in categories X/Y, years 2000–2003, B with `estimated=true` in 2001, D only in 2002 (single point). Fixture `video.toml`: `reviewed = true`, `seconds_per_year = 0.5`.

- [ ] **Step 2: Write failing unit tests** (in `src/data.rs` and `src/validate.rs` `#[cfg(test)]`):

```rust
#[test] fn loads_fixture() {
    let d = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
    assert_eq!(d.items.len(), 4);
    assert_eq!((d.first_year, d.last_year), (2000, 2003));
    assert!(d.items.iter().all(|i| i.points.windows(2).all(|w| w[0].year < w[1].year)));
}
#[test] fn quoted_names_with_commas() { /* tempdir CSV row: "Korea, South",Asia,,2000,1,,false → name == "Korea, South" */ }
#[test] fn duplicate_name_year_is_error() { /* tempdir with two rows A,2000 → run() errors contain "duplicate" */ }
#[test] fn missing_image_is_error() { /* image=images/nope.png → errors contain "nope.png" */ }
#[test] fn bad_number_is_error_with_line() { /* value=abc on CSV line 3 → error contains "line 3" */ }
#[test] fn report_summary() {
    let r = run(Path::new("tests/fixtures/week")).unwrap();
    assert!(r.errors.is_empty()); assert!(r.reviewed);
    assert!(r.estimated_share > 0.0 && r.estimated_share < 1.0);
}
```
Use `std::env::temp_dir()` plus a unique subfolder for the temporary cases (no tempfile crate).

- [ ] **Step 3:** Run `cargo test`. Expected: FAIL (types not defined).
- [ ] **Step 4:** Implement the interfaces above. Required CSV columns: `name,category,image,year,value,source,estimated`. An empty `image` → `None`. `estimated` accepts `true/false/1/0/yes/no` (case-insensitive). The `validate` warnings: `reviewed = false`, an item with only one point, an item with no `source` and not estimated.
- [ ] **Step 5:** Run `cargo test`. Expected: PASS. Then run `cargo run -- validate tests/fixtures/week`. Expected: a summary with 4 items, 2000–2003, an estimated %, reviewed: yes, exit 0.
- [ ] **Step 6:** Commit with the message `feat: data loading, config and validate command`.

---

### Task 2: Interpolation and ranking (`scene.rs`)

**Files:** Create `src/scene.rs`

**Interfaces:**
- Consumes: `data::{Dataset, Item, Point}`
- Produces:
  - `scene::value_at(item: &Item, year: f64) -> Option<(f64, bool /*estimated*/)>`: linear interpolation between points. Over `[first-1, first)` it ramps from 0 to the first value; over `(last, last+1]` it ramps from the last value to 0; outside that range → `None`. `estimated` is true if either neighbouring point is estimated. Gaps inside the range are interpolated straight across.
  - `scene::BarState { idx: usize /*into Dataset.items*/, value: f64, slot: f32 /*0 = top, eased*/, alpha: f32, estimated: bool }`
  - `scene::FrameState { year: f64, bars: Vec<BarState> /*top_n+1 visible, sorted by slot*/, axis_max: f64 }`
  - `scene::RankTracker::new(n_items: usize, top_n: usize) -> RankTracker`; `RankTracker::step(&mut self, ds: &Dataset, year: f64, dt: f64) -> FrameState`

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn interpolates_linearly()      { /* A: 2000→10, 2001→20; value_at(A, 2000.5) == Some((15.0, false)) */ }
#[test] fn ramps_in_and_out()           { /* D only at 2002=8: at 2001.5 → 4.0; at 2002.5 → 4.0; at 2000.9 → None; at 2003.1 → None */ }
#[test] fn gap_interpolated_across()    { /* points 2000=0, 2003=30 → value_at(2001.0) == 10.0 */ }
#[test] fn estimated_propagates()       { /* B est at 2001 → value_at(B, 2000.5).1 == true */ }
#[test] fn ties_stable_by_name() {
    // two items with equal values every year; step 100 frames → order (by slot) never changes, "Alpha" above "Beta"
}
#[test] fn slot_eases_not_jumps() {
    // overtake between frames: after one 1/60 s step slot changed by < 0.5; after 0.6 s of steps |slot - target| < 0.05
}
#[test] fn axis_max_tracks_leader() { /* after 1 s of steps at a constant year, axis_max within 2% of the top value */ }
```

- [ ] **Step 2:** Run `cargo test scene`. Expected: FAIL.
- [ ] **Step 3: Implement.** Rank = sort by value desc, then name asc. Slot and axis easing use exponential smoothing, `x += (target - x) * (1 - (-dt / TAU).exp())`, with `TAU = 0.12` s for slots and `0.25` s for `axis_max`. Bars whose target slot is ≥ `top_n` ease toward slot `top_n` (just below the frame) and fade `alpha` to 0. A new item starts at slot `top_n`. `FrameState.bars` contains only bars with `alpha > 0.01` or `slot < top_n`.
- [ ] **Step 4:** Run `cargo test scene`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: interpolation and eased ranking`.

---

### Task 3: Timeline (`timeline.rs`)

**Files:** Create `src/timeline.rs`

**Interfaces:**
- Consumes: `config::VideoConfig`, `data::Dataset`
- Produces:
  - `timeline::Format { Main, Short }`
  - `timeline::Phase { Intro { t: f64 /*0..1*/ }, Race { year: f64 }, Hold, End { t: f64 } }`
  - `timeline::Timeline { fps: u32, start_year: i32, end_year: i32, intro: f64, race: f64, hold: f64, end: f64 }`; `Timeline::new(fmt: Format, cfg: &VideoConfig, ds: &Dataset, fps: u32) -> Timeline`; `total_seconds(&self) -> f64`; `frame_count(&self) -> u64`; `phase_at(&self, frame: u64) -> Phase`

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn main_duration() { /* 1960–2026, spy 3.5 → race 231.0, total 241.0, frames at 60fps == 14460 */ }
#[test] fn short_full_range_when_readable() { /* 1960–2026, short 58 → race 55.0; 55/66 ≥ 0.8 → start_year 1960 */ }
#[test] fn short_trims_start_year() { /* 1900–2000, short 58 → race 55; floor(55/0.8)=68 → start_year 1932 */ }
#[test] fn phases_in_order() {
    // Main fixture: frame 0 → Intro{t:0}; first race frame → Race{year == start_year as f64};
    // last race frame year ≤ end_year; then Hold; last frame → End{t≈1}
}
```

- [ ] **Step 2:** Run `cargo test timeline`. Expected: FAIL.
- [ ] **Step 3: Implement.** Main: `intro=3, hold=3, end=4, race=years*seconds_per_year`. Short: `intro=1.5, hold=1.5, end=0, race=short_seconds-3`. If `race/years < 0.8`, set `start_year = end_year - floor(race/0.8)`. During Race, `year = start_year + (t_in_race / race) * (end_year - start_year)`.
- [ ] **Step 4:** Run `cargo test timeline`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: timeline for main and short formats`.

---

### Task 4: Number formatting, text, layout, drawing one frame

**Files:** Create `src/fmt.rs`, `src/text.rs`, `src/layout.rs`, `src/draw.rs`, `assets/fonts/Inter-Regular.ttf`, `assets/fonts/Inter-Bold.ttf`

**Interfaces:**
- Consumes: `scene::FrameState`, `timeline::Phase`, `config::{VideoConfig, ChannelConfig}`, `data::Dataset`
- Produces:
  - `fmt::format_value(v: f64, prefix: &str, suffix: &str) -> String`
  - `text::Text::new() -> Text` (FontSystem with system fonts + bundled Inter); `Text::draw(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align)`; `Text::measure(&mut self, s: &str, size: f32, bold: bool) -> f32` (width); `text::Align { Left, Right, Center }`
  - `layout::Layout { width: u32, height: u32, top_n: usize, chart_left, chart_top, chart_right, chart_bottom: f32, bar_gap: f32 /*fraction*/, title_size, subtitle_size, label_size, year_size: f32, year_pos: (f32, f32), watermark_pos: (f32, f32) }`; `Layout::landscape(scale: f32) -> Layout`; `Layout::portrait(scale: f32, top_n: usize) -> Layout` (scale 1.0 = full res, 0.5 = preview)
  - `draw::Painter::new(ds: &Dataset, cfg: &VideoConfig, ch: &ChannelConfig, layout: Layout) -> anyhow::Result<Painter>` (assigns category colours; image loading arrives in Task 6); `Painter::paint(&mut self, pm: &mut Pixmap, phase: &Phase, fs: &FrameState)`

- [ ] **Step 1: Get the fonts.** Download the Inter 4.1 release zip from `https://github.com/rsms/inter/releases` and copy `extras/ttf/Inter-Regular.ttf` and `Inter-Bold.ttf` into `assets/fonts/`, keeping the zip's `LICENSE.txt` as `assets/fonts/OFL.txt`. If there's no network, copy `DejaVuSans.ttf` and `DejaVuSans-Bold.ttf` from `/usr/share/fonts` under the Inter file names and note that in the commit. Load them with `include_bytes!`.

- [ ] **Step 2: Write failing tests:**

```rust
// fmt.rs
assert_eq!(format_value(0.5, "$", "B"), "$0.5B");
assert_eq!(format_value(99.94, "", ""), "99.9");
assert_eq!(format_value(1234.5, "", ""), "1,235");
assert_eq!(format_value(1234567.0, "$", ""), "$1,234,567");
assert_eq!(format_value(2.4e12, "", ""), "2,400,000,000,000");
assert_eq!(format_value(-5.0, "", ""), "-5.0");
// draw.rs
#[test] fn paints_bar_in_category_color() {
    // fixture, landscape(0.5), Race year 2001.0, FrameState from RankTracker after 2 s of steps;
    // pixel at the horizontal middle of the top bar's row == the category colour (±8 per channel)
}
#[test] fn fewer_items_than_top_n() { /* top_n 10, fixture has 4 → paint() doesn't panic */ }
#[test] fn non_latin_names_draw() {
    // Text::draw "中国 São Tomé Côte d'Ivoire" on a blank pixmap → ≥ 50 non-background pixels, no panic
}
#[test] fn negative_value_zero_length_bar() { /* value -5 → no bar pixels in that row beyond chart_left+2, label still drawn */ }
```

- [ ] **Step 3:** Run `cargo test`. Expected: FAIL.
- [ ] **Step 4: Implement.** Text uses `cosmic_text::{FontSystem, SwashCache, Buffer}` and paints the glyph rects from `buffer.draw(...)` into the pixmap with `fill_rect`. Cache shaped buffers in a `HashMap<(String, u32 size, bool), Buffer>`, because labels repeat every frame. Palette (auto by first-seen category): `#38bdf8 #f472b6 #facc15 #4ade80 #fb923c #a78bfa #f87171 #2dd4bf #e879f9 #a3e635 #60a5fa #fbbf24`; `cfg.colors` overrides it. Race frame contents: title and subtitle top-left; bars with rounded ends (radius = half the bar height); name label inside the bar's left end, or right of the bar if it doesn't fit; value label right of the bar, plus `*` if estimated; year (`year.floor()`) large at `year_pos`, 25% alpha white; watermark `channel.name` at `watermark_pos`, 40% alpha. Intro, Hold and End can paint a plain race frame for now (cards arrive in Task 6).
- [ ] **Step 5:** Run `cargo test`. Expected: PASS.
- [ ] **Step 6:** Commit with the message `feat: text, layout and frame painting`.

---

### Task 5: Encoder and `render` command (silent landscape). Milestone 1

**Files:** Create `src/encode.rs`, `src/render.rs`, `tests/cli.rs`; Modify `src/main.rs`

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces:
  - `encode::Encoder::start(out: &Path, w: u32, h: u32, fps: u32, audio: Option<AudioSpec>) -> anyhow::Result<Encoder>`; `write_frame(&mut self, rgba: &[u8]) -> anyhow::Result<()>`; `finish(self) -> anyhow::Result<()>`
  - `encode::AudioSpec { path: PathBuf, duration: f64 }` (used in Task 7)
  - `render::render(week_dir: &Path, fmt: Format, preview: bool) -> anyhow::Result<PathBuf>`: writes `<week>/out/main.mp4` or `<week>/out/short.mp4`
  - CLI `barrace render <DIR> [--format main|short|both] [--preview]` (`short` and `both` return "not yet" until Task 8)

- [ ] **Step 1: Write failing tests** in `tests/cli.rs` (run the built binary via `env!("CARGO_BIN_EXE_barrace")`; fixture spy 0.5 → main = 3+1.5+3+4 = 11.5 s):

```rust
#[test] fn render_main_preview() {
    // copy the fixture to a temp dir; `barrace render <tmp> --preview` exits 0;
    // ffprobe width,height == 960,540; r_frame_rate == 30/1; duration within 0.1 s of 11.5
}
#[test] fn render_refuses_unreviewed() { /* reviewed=false → exit != 0, stderr contains "reviewed" */ }
#[test] fn missing_ffmpeg_clear_error() { /* PATH="" → exit != 0, stderr contains "ffmpeg not found" */ }
```

- [ ] **Step 2:** Run `cargo test --test cli`. Expected: FAIL.
- [ ] **Step 3: Implement.** The ffmpeg args: `-y -f rawvideo -pix_fmt rgba -s {w}x{h} -r {fps} -i - -c:v libx264 -preset medium -crf 18 -pix_fmt yuv420p -movflags +faststart {out}`. If spawning fails with NotFound, return `"ffmpeg not found on PATH"`. The render loop: for each frame, `phase_at` → for `Race`, `tracker.step(ds, year, 1/fps)`; for other phases, step at the clamped year so the bars hold still → `painter.paint` → `write_frame(pm.data())`. The background is opaque, so tiny-skia's premultiplied RGBA equals straight RGBA. Print progress every 10%.
- [ ] **Step 4:** Run `cargo test --test cli`. Expected: PASS. Then play `tests/fixtures/week/out/main.mp4` by hand or grab a frame with `ffmpeg -ss 5 -i … -frames:v 1 /tmp/f.png`, and confirm the bars, labels and year look right.
- [ ] **Step 5:** Commit with the message `feat: render command produces landscape mp4`.

---

### Task 6: Images, axis, intro/end cards

**Files:** Modify `src/draw.rs`, `src/layout.rs`

**Interfaces:**
- Consumes: `Painter` (Task 4), `Dataset::estimated_share` (Task 1)
- Produces: the same `Painter` API. `Painter::new` now also loads images; on a load failure it warns to stderr and uses the initials fallback.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn image_drawn_at_bar_tip() { /* write a solid-red 64x64 PNG to a temp dir, item.image = it; pixel at the bar tip centre ≈ red */ }
#[test] fn initials_fallback() { /* image None → circle at the bar tip has the category colour darkened, ≥ 20 text pixels inside */ }
#[test] fn intro_card_shows_title() { /* Phase::Intro{t:0.5} → ≥ 500 text-coloured pixels in the centre third */ }
#[test] fn end_card_estimated_note() { /* fixture has estimates → paint(End{t:0.5}) → painter.last_strings contains a string starting with "* estimated values:" */ }
```
(For testability: `Painter` keeps `pub last_strings: Vec<String>`, the strings drawn in the last `paint`.)

- [ ] **Step 2:** Run `cargo test draw`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - **Images:** load via `image::open`, resize to the bar height with `image::imageops::resize` (Lanczos3), convert to a premultiplied `Pixmap`, cache per item, and draw clipped to a circle (a `Mask` from a circle path) at the right end of the bar.
  - **Initials fallback:** the first letters of up to 2 words.
  - **Axis:** 4–5 nice ticks (1/2/5 × 10^k) up to `axis_max`, gridlines in white at 10% alpha, tick labels via `format_value` above the chart.
  - **Intro:** the race frame dimmed 70%, with the title (bold, 2× title size) and subtitle centred, fading in over `t` 0–0.3.
  - **End (main only):** dimmed background, "Sources" heading, up to 5 distinct source domains from `Item.sources`, `* estimated values: NN%` when the share is > 0, and the channel name.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Render a preview and look at the frames at 1 s, 5 s and 10 s.
- [ ] **Step 5:** Commit with the message `feat: images, axis, intro and end cards`.

---

### Task 7: Music

**Files:** Modify `src/encode.rs`, `src/render.rs`; Test `tests/cli.rs`

**Interfaces:**
- Consumes: `AudioSpec` (Task 5)
- Produces: `render::pick_music(cfg: &VideoConfig, week_dir: &Path, music_dir: &Path) -> Option<PathBuf>`. `cfg.music` is resolved against the week dir. If it isn't set, take the files in `music_dir` (`./music`) with mp3/m4a/wav/ogg/flac extensions, sorted, and pick index = (FNV-1a hash of `cfg.title`) % len. Returns None if there are none.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn render_with_music_has_audio() {
    // ffmpeg -f lavfi -i sine=f=440:d=5 tmp/music/t.mp3 (shorter than the video → must loop);
    // render with cwd=tmp → ffprobe shows an audio stream with duration within 0.15 s of the video duration
}
#[test] fn no_music_is_silent_ok() { /* empty music dir → exit 0, no audio stream, stderr warns "no music" */ }
#[test] fn pick_is_deterministic() { /* same title + same dir → same file twice */ }
```

- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3: Implement.** With audio, insert `-stream_loop -1 -i {music}` after the video input, then add `-map 0:v -map 1:a -af afade=t=in:d=2,afade=t=out:st={dur-3}:d=3 -c:a aac -b:a 192k -t {dur}`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: music mux with fades`.

---

### Task 8: Portrait layout and Short format

**Files:** Modify `src/layout.rs`, `src/render.rs`, `src/main.rs`, `src/draw.rs`; Test `tests/cli.rs`

**Interfaces:**
- Consumes: `Layout::portrait`, `Timeline::new(Format::Short, …)`
- Produces: `render(week_dir, Format::Short, preview)` → `out/short.mp4`; `--format both` renders main, then short.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn render_short_preview() { /* short_seconds 12 in the temp fixture → 540x960, duration ≈ 12.0 ±0.1 */ }
#[test] fn render_both() { /* --format both → main.mp4 and short.mp4 both exist */ }
```

- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3: Implement.** Portrait layout: title block in the top 14%, the year counter centred below the chart, the chart from 18% to 82% of the height, at least 6% side margins, and the bottom 12% left clear for the Shorts UI. Text sizes are about 1.3× landscape at equal scale. `top_n` comes from `cfg.top_n_short`. Short has no End phase (the timeline already sets `end = 0`).
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Look at the Short frames by hand.
- [ ] **Step 5:** Commit with the message `feat: vertical short format`.

---

### Task 9: Thumbnail and sample week

**Files:** Modify `src/render.rs`, `src/main.rs`, `src/draw.rs`; Create `weeks/2026-11-23-sample/data.csv`, `weeks/2026-11-23-sample/video.toml`, `README.md`

**Interfaces:**
- Produces: `render::thumb(week_dir: &Path, year: Option<f64>) -> anyhow::Result<PathBuf>` → `out/thumb.png`, 1280×720; `Painter::paint_thumb_overlay(&mut self, pm: &mut Pixmap)`. CLI `barrace thumb <DIR> [--year Y]` (default: the last year).

- [ ] **Step 1: Write a failing test:** `thumb_png_1280x720` → the file exists, `image::image_dimensions == (1280, 720)`.
- [ ] **Step 2:** Run it. Expected: FAIL.
- [ ] **Step 3: Implement.** Render the landscape layout at scale 1.0. Ease the tracker for 2 s at the chosen year so it settles. Paint, then paint the overlay: the title in bold at 2.2× size with a dark 60%-alpha band behind it in the top third. Downscale with `image` Lanczos3.
- [ ] **Step 4: Sample week.** Write a real but small dataset: the top 15 countries by population 1990–2023 (World Bank `SP.POP.TOTL`, typed in or fetched once by hand), with `reviewed = true`. In `README.md`, cover install (`cargo install --path .`), the weekly flow from the spec, the CSV and TOML formats, and dropping tracks into `music/`.
- [ ] **Step 5:** Run `cargo test`, then `cargo run --release -- render weeks/2026-11-23-sample --format both` and `cargo run --release -- thumb weeks/2026-11-23-sample`. Expected: three files in `out/`, and the full-res main renders in under 5 minutes (record the time in the commit message).
- [ ] **Step 6:** Commit with the message `feat: thumbnail command, sample week, README`.
