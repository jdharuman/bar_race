# barrace v2 Rich Videos Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make barrace videos feel faster and richer: sub-year data, pace by era, full-digit numbers, fact captions, a leader badge, bar colours taken from images, a light theme, a Like/Subscribe end card, 12-bar Shorts, and bundled CC0 music.

**Architecture:** Extends the existing modules. Time becomes `f64` everywhere (`data`, `scene`, `timeline`). A new `events.rs` loads captions. `scene::Crossfade` drives both the caption and the leader badge. `draw.rs` gains a theme, the badge, captions and the end card. The CLI gains render options.

**Tech Stack:** unchanged (Rust, tiny-skia, cosmic-text, image, csv, serde, toml, clap, anyhow, ffmpeg).

**Spec:** `docs/superpowers/specs/2026-11-21-rich-videos-design.md` (extends `2026-11-20-barrace-design.md`)

## Global Constraints

- Time strings: `1999`, `1999-02` (monthly), `1999-Q2` (quarterly) → `1999.0`, `1999 + 1/12`, `1999.25`.
- Counter label uses the finest granularity in the data: `1999` / `Feb 1999` / `Q2 1999`.
- Pace is seconds per calendar year. Default when unset: yearly 2.0, quarterly 6.0, monthly 12.0.
- Main timeline: intro 3 s, race, hold 3 s, end card 5 s. Short: intro 1.5 s, race, hold 1.0 s, end card 2.5 s, total = `short_seconds`.
- Short readable floor: 0.8 s/year for yearly data, 3.0 s/year for sub-year data. Below the floor, the start moves to a later whole year.
- Grow-in/shrink-out ramp lasts one data step (`Dataset::step`).
- `number_style` and `bar_colors` live in `video.toml`; `theme` lives in `channel.toml`.
- Defaults: `number_style = "full"`, `bar_colors = "image"`, `theme = "light"`, channel `name = ""`, `top_n_short = 12`.
- Caption and badge fades: 0.3 s and 0.4 s.
- End card: no YouTube logo or trademarked art; Like + Subscribe + bell drawn from vector paths.
- Music: only CC0/public-domain tracks, each listed in `music/CREDITS.md`.
- Render speed: a 4-minute main video at 1080p60 in about 5 minutes.
- v1 week folders render without edits.

## Review Focus

1. **Mixed granularity in one CSV** (some `1999`, some `1999-03`): rows parse (a bare year = January), labels use the finest granularity, no panic. Tested in Task 1.
2. **Bad pace sections** (`from >= to`, overlapping sections, sections outside the data range): `from >= to` and overlaps are config errors; parts outside the data range are ignored. Tested in Task 2.
3. **Bad `events.csv`** (missing file, `from >= to`, unparseable time): a missing file means no captions; bad rows are validate errors with line numbers. Tested in Task 6.
4. **Leader flipping back and forth every few frames** (near-ties): the badge crossfade never shows two badges at full opacity and never blinks. Tested in Task 6.
5. **Very long caption text or a mostly-white flag**: the caption shrinks to fit its box; a flag with no dominant colour falls back to the category palette. Tested in Tasks 6 and 5.

---

### Task 1: Sub-year time model

**Files:** Modify `src/data.rs`, `src/scene.rs`, `src/validate.rs`, `src/fmt.rs`, `src/timeline.rs` (only the type changes needed to compile), `src/render.rs`, `src/draw.rs` (counter label), `src/main.rs` (validate printout)

**Interfaces:**
- Produces:
  - `data::Granularity { Year, Quarter, Month }` (derive `PartialOrd, Ord`: Year < Quarter < Month = finer)
  - `data::parse_time(s: &str) -> Option<(f64, Granularity)>`
  - `data::Point { t: f64, value: f64, estimated: bool }` (renamed from `year: i32`)
  - `data::Dataset { items, first: f64, last: f64, granularity: Granularity }`; `Dataset::step(&self) -> f64` (1.0, 0.25, 1/12)
  - `fmt::format_time(t: f64, g: Granularity) -> String`
  - `scene::value_at(item: &Item, t: f64, step: f64) -> Option<(f64, bool)>`. The ramp spans `step` instead of 1.0. `RankTracker::step(ds, t, dt)` passes `ds.step()`.
  - `validate::Report { first: f64, last: f64, granularity: Granularity, .. }`; the CLI prints `years: <format_time(first)>–<format_time(last)>`.

- [ ] **Step 1: Write failing tests:**

```rust
// data.rs
assert_eq!(parse_time("1999"), Some((1999.0, Granularity::Year)));
assert_eq!(parse_time("1999-02"), Some((1999.0 + 1.0 / 12.0, Granularity::Month)));
assert_eq!(parse_time("1999-Q2"), Some((1999.25, Granularity::Quarter)));
assert_eq!(parse_time("1999-13"), None); assert_eq!(parse_time("1999-Q5"), None); assert_eq!(parse_time("abc"), None);
#[test] fn mixed_granularity_uses_finest() { /* rows A 2000, A 2000-06 → granularity Month, step 1/12, points sorted by t */ }
#[test] fn duplicate_same_time_different_spelling() { /* "2000" and "2000-01" for the same name → "duplicate" error */ }
// fmt.rs
assert_eq!(format_time(1999.0, Granularity::Year), "1999");
assert_eq!(format_time(1999.0 + 1.0 / 12.0 + 0.01, Granularity::Month), "Feb 1999");
assert_eq!(format_time(1999.26, Granularity::Quarter), "Q2 1999");
assert_eq!(format_time(1999.999, Granularity::Year), "1999");
// scene.rs
#[test] fn monthly_ramp_is_one_month() { /* only point 2000-06=12, step 1/12: at t(2000-05)+1/24 value 6.0; at t(2000-05) None */ }
```
Update the existing tests for the renamed fields (`year` → `t`, `first_year` → `first`). They must still pass with step 1.0.

- [ ] **Step 2:** Run `cargo test`. Expected: compile errors on the new names → FAIL.
- [ ] **Step 3: Implement.** A month is detected with `^\d{4}-(\d{2})$` (01–12) and a quarter with `^\d{4}-Q([1-4])$`, case-insensitive, written by hand (no regex crate). The duplicate key is the time rounded to 1e-6. `format_time` takes `floor` with an epsilon of 1e-6 so float drift (e.g. 1999.9999999) still lands in the right month. `Timeline` and `render` switch to `f64` times (`start_year`/`end_year` become `start`/`end: f64`; `Phase::Race { year }` becomes `Phase::Race { t }`). Painter draws the counter with `format_time(fs.year, ds.granularity)`; rename `FrameState.year` to `FrameState.t`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS, all existing tests green.
- [ ] **Step 5:** Commit with the message `feat: monthly and quarterly time in data and counter`.

---

### Task 2: Pace sections and the new timeline

**Files:** Modify `src/config.rs`, `src/timeline.rs`

**Interfaces:**
- Consumes: `Dataset { first, last, granularity }`, `Dataset::step`
- Produces:
  - `config::PaceSection { from: f64, to: f64, seconds_per_year: f64 }` (TOML ints or floats)
  - `VideoConfig.seconds_per_year: Option<f64>`, `VideoConfig.pace: Vec<PaceSection>`; `problems()` reports `from >= to`, overlapping sections, and non-positive pace. `short_seconds` range is now 8–180.
  - `timeline::default_pace(g: Granularity) -> f64` (2.0 / 6.0 / 12.0)
  - `Timeline { fps, start: f64, end: f64, intro, race, hold, outro: f64, segs: Vec<(f64 /*t0*/, f64 /*t1*/, f64 /*seconds*/)> }`; `new(fmt, cfg, ds, fps)`, `total_seconds`, `frame_count`, `phase_at(frame) -> Phase`; `Phase::{Intro{t}, Race{t}, Hold, End{t}}`. `End` now also occurs in Short.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn default_pace_by_granularity() { /* yearly 1960–2026 no setting → race 132.0; total 3+132+3+5 = 143.0 */ }
#[test] fn pace_sections_apply() {
    // 1600–2000 yearly, base 1.7, section 1600–1800 at 0.45 → race = 200*0.45 + 200*1.7 = 430.0;
    // the race-phase frame at intro + 90.0 s has t == 1800.0 (±1e-6)
}
#[test] fn sections_outside_data_ignored() { /* section 1500–1550 on 1600–2000 data → same race as without it */ }
#[test] fn overlapping_sections_rejected() { /* 1600–1800 and 1700–1900 → problems() mentions "overlap" */ }
#[test] fn short_scales_sections_and_has_outro() {
    // same data as pace_sections_apply, short 58 → race 53.0, outro 2.5, total 58.0;
    // the ratio of section time to rest stays 90:340 (unless the floor trimmed it)
}
#[test] fn short_floor_trims_to_whole_year() { /* yearly 1900–2000, short 58: race 53, 53/100 < 0.8 → start = 2000 - floor(53/0.8) = 1934.0 */ }
#[test] fn short_floor_subyear() { /* monthly 1996-01..2026-01, short 58: 53/30 = 1.77 < 3.0 → start = 2026 - floor(53/3) = 2009.0 */ }
```
Update `main_duration` (hold 3, end card 5) and `phases_in_order`. `single_year_never_races_past_end` must still pass.

- [ ] **Step 2:** Run `cargo test timeline config`. Expected: FAIL.
- [ ] **Step 3: Implement.** Build `segs` by cutting `[start, end]` at the section edges. Each piece's seconds = length × (that section's pace, or the base pace). Base = `cfg.seconds_per_year.unwrap_or(default_pace(g))`. Short: compute segs over the full range, apply the floor (avg = 53 / (end − start)), recompute segs over the trimmed range, then scale every piece by `race / sum`. `phase_at` in Race walks `segs`, accumulating seconds, and interpolates t linearly inside the piece. A single-year span has `segs` = `[(end, end, race)]` with t fixed at `end`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: pace sections and new main/short timelines`.

---

### Task 3: Number styles

**Files:** Modify `src/fmt.rs`, `src/config.rs`, `src/draw.rs`

**Interfaces:**
- Produces: `fmt::NumberStyle { Full, Compact }` (serde `rename_all = "lowercase"`), `VideoConfig.number_style: NumberStyle` (default `Full`); `fmt::format_value(v, prefix, suffix, style: NumberStyle) -> String`. Both the value labels and the axis use `cfg.number_style`.

- [ ] **Step 1: Write failing tests:**

```rust
use NumberStyle::*;
assert_eq!(format_value(77_246_849_166.4, "$", "", Full), "$77,246,849,166");
assert_eq!(format_value(999.94, "", "", Full), "999.9");
assert_eq!(format_value(1000.4, "", "", Full), "1,000");
assert_eq!(format_value(-1500.0, "$", "", Full), "-$1,500");
assert_eq!(format_value(0.5, "", "", Full), "0.5");
assert_eq!(format_value(950.0, "", "", Compact), "950");
assert_eq!(format_value(1_234.0, "", "", Compact), "1.2K");
assert_eq!(format_value(3_400_000.0, "$", "", Compact), "$3.4M");
assert_eq!(format_value(2.4e12, "", "", Compact), "2.4T");
assert_eq!(format_value(99.96, "", "", Full), "100.0");   // < 1000 keeps one decimal
```

- [ ] **Step 2:** Run `cargo test fmt`. Expected: FAIL.
- [ ] **Step 3: Implement.** Full: `|v| < 1000` → one decimal, otherwise round to an integer with commas. Compact: below 1000 → integer; otherwise one decimal with K/M/B/T. Update callers.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: full-digit and compact number styles`.

---

### Task 4: Light theme and no default channel name

**Files:** Modify `src/config.rs`, `src/draw.rs`, `channel.toml`

**Interfaces:**
- Produces: `config::Theme { Light, Dark }` (lowercase serde, default `Light`); `ChannelConfig { name: String /* "" */, logo, theme: Theme, background: Option<String>, text: Option<String>, accent: String }`; `ChannelConfig::colors(&self) -> (bg_hex: String, text_hex: String)`: Light `#ffffff`/`#1f2937`, Dark `#0f172a`/`#ffffff`, overridden by `background`/`text`.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn light_theme_default_white() { /* Painter with ChannelConfig::default(): pixel (2,2) after paint == white */ }
#[test] fn dark_theme_navy() { /* theme Dark → pixel (2,2) == #0f172a */ }
#[test] fn empty_name_draws_no_watermark() { /* default channel: last_strings has no "Stat Race" and no empty string drawn */ }
#[test] fn initials_readable_on_light() { /* initials text colour stays white on the darkened circle under the light theme */ }
```

- [ ] **Step 2:** Run `cargo test draw config`. Expected: FAIL.
- [ ] **Step 3: Implement.** Gridlines use the text colour at 12% (light) / 10% (dark); the year counter uses the text colour at 35% (light) / 25% (dark). Labels inside bars stay white with any theme; labels outside bars use the text colour. Skip the watermark and the end-card name when `name` is empty. In `channel.toml`: `name = ""`, `theme = "light"`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Render the fixture preview and look at a frame.
- [ ] **Step 5:** Commit with the message `feat: light theme default, channel name optional`.

---

### Task 5: Bar colours from images

**Files:** Modify `src/draw.rs`, `src/config.rs`

**Interfaces:**
- Produces: `config::BarColors { Image, Category }` (lowercase, default `Image`), `VideoConfig.bar_colors`; `draw::dominant_color(img: &image::RgbaImage) -> Option<[u8; 3]>`.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn dominant_ignores_white() { /* 10x10: 6 white, 4 red → Some([255,0,0]) */ }
#[test] fn all_white_none() { /* white image → None */ }
#[test] fn transparent_ignored() { /* 8 transparent blue + 2 opaque green → Some(green) */ }
#[test] fn bar_uses_image_colour() { /* fixture-like item with a solid green PNG, bar_colors Image → bar row has green pixels */ }
#[test] fn bar_falls_back_to_category() { /* white PNG → palette colour; bar_colors Category → palette colour even with a green PNG */ }
```

- [ ] **Step 2:** Run `cargo test draw`. Expected: FAIL.
- [ ] **Step 3: Implement.** Quantise each opaque pixel (alpha ≥ 128) to 4 bits per channel. Skip near-white (all channels > 225) and near-black (all < 30). Pick the most frequent bucket and return the mean of its pixels. Run this on the original image once, in `Painter::new`. `cfg.colors[category]` still overrides.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: bar colours from item images`.

---

### Task 6: Fact captions and leader badge

**Files:** Create `src/events.rs`; Modify `src/scene.rs`, `src/layout.rs`, `src/draw.rs`, `src/render.rs`, `src/validate.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `data::parse_time`
- Produces:
  - `events::Event { from: f64, to: f64, headline: String, big: String, caption: String }`; `events::parse(week_dir: &Path) -> Result<(Vec<Event>, Vec<String> /*errors*/)>` (missing file → empty, no error); `events::active(events: &[Event], t: f64) -> Option<usize>` (the latest `from` among those with `from <= t < to`)
  - `scene::Crossfade::new(fade: f64) -> Crossfade`; `step(&mut self, target: Option<usize>, dt: f64) -> Option<(usize, f32)>`. The shown item fades out fully before the next one fades in; each half takes `fade / 2` seconds.
  - `scene::FrameState.leader: Option<usize>` (rank-1 item index)
  - `draw::Overlay { caption: Option<(usize, f32)>, leader: Option<(usize, f32)> }`; `Painter::new(ds, cfg, ch, layout, events: Vec<Event>)`; `Painter::paint(pm, phase, fs, ov: &Overlay)`
  - `layout::Layout::{caption_box, badge_box, year_box}(&self) -> (f32, f32, f32, f32)` (x, y, w, h); `Layout.portrait: bool` (replaces `year_centered`)
  - validate: `events.csv` errors are appended to `Report.errors`

- [ ] **Step 1: Write failing tests:**

```rust
// events.rs
#[test] fn parses_events() { /* "from,to,headline,big,caption\n1913,1925,US produces,90%,World's Oil\n" → 1 event, from 1913.0 */ }
#[test] fn bad_rows_reported_with_line() { /* from >= to on line 2 → error contains "line 2"; bad time on line 3 → "line 3" */ }
#[test] fn missing_file_is_empty() { /* no events.csv → Ok((vec![], vec![])) */ }
#[test] fn later_from_wins() { /* [1900–1950), [1920–1930) → active(1925) == Some(1), active(1935) == Some(0), active(1950) == None */ }
// scene.rs
#[test] fn crossfade_switches_through_zero() {
    // target 0 for 1 s → (0, 1.0); target 1 → alpha falls; it never returns Some((1, a)) while 0 still has a > 0
}
#[test] fn crossfade_flip_flop_no_double() { /* alternate targets 0/1 every frame for 2 s → each step returns at most one item, alpha ≤ 1 */ }
#[test] fn leader_is_top_rank() { /* fixture at 2001 → leader == Beta's index */ }
// layout.rs
#[test] fn overlay_boxes_clear_of_chart_and_each_other() {
    // for landscape(1.0,10) and portrait(1.0,12): caption, badge and year boxes are inside the frame,
    // don't intersect the chart rect (chart_left..chart_right+label room, chart_top..chart_bottom) and don't intersect each other
}
// draw.rs
#[test] fn caption_drawn_when_active() { /* Overlay caption Some((0,1.0)) → last_strings contains "US produces", "90%", "World's Oil" */ }
#[test] fn long_caption_fits_box() { /* 80-char caption → no text pixels outside caption_box (±2 px) */ }
#[test] fn badge_drawn_for_leader() { /* leader Some((idx,1.0)) with a red PNG → badge_box centre pixel ≈ red */ }
```

- [ ] **Step 2:** Run `cargo test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - **Landscape boxes:** a right column centred near x = 0.87 W. Caption box at y 0.22–0.44 H, badge box at y 0.46–0.64 H (square, side 0.16 H), year box as today (bottom right). Shrink `chart_right` and the label room if the boxes need it.
  - **Portrait boxes:** chart from 0.15 to 0.62 H with 12 bars. Caption box at 0.64–0.73 H, full width minus margins. Badge box (left) and year box (right) side by side at 0.75–0.86 H. The bottom 12% stays clear.
  - **Caption drawing:** `headline` bold in red `#e11d48`, `big` bold at 2.4× the headline size, `caption` regular; each line fitted to the box width with `fit_size`.
  - **Badge:** the leader's image as a large circle (cache a second, bigger circle image per item), or the initials circle. Alpha comes from the Overlay.
  - **Render loop:** keeps `Crossfade::new(0.3)` for the caption (target = `events::active(events, t)`) and `Crossfade::new(0.4)` for the leader (target = `fs.leader`), stepping both with `dt`. During intro, hold and end, the caption target is evaluated at the clamped t. The thumbnail shows the badge but no caption.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Render a fixture preview with an `events.csv` and look at a caption frame.
- [ ] **Step 5:** Commit with the message `feat: fact captions and leader badge`.

---

### Task 7: Like / Subscribe / bell end card

**Files:** Modify `src/draw.rs`; Test in `src/draw.rs`

**Interfaces:**
- Produces: `draw::EndState { pop: f32 /*0..1 scale*/, liked: bool, subscribed: bool, bell_angle: f32 /*radians*/, cursor: Option<(f32, f32)> /*fraction of the button row: 0 = Like centre, 1 = Subscribe centre*/ }`; `draw::end_state(t: f64) -> EndState`.

Timeline within `End { t }` (t from 0 to 1): pop-in over 0–0.15 (ease-out-back); the cursor glides to Like over 0.15–0.35 and clicks at 0.35; the cursor glides to Subscribe over 0.35–0.55 and clicks at 0.55; the cursor fades out by 0.65; the bell swings `0.35 rad × sin(2π · 2 · u) × (1 − u)` for u over 0.6–0.9, and is 0 elsewhere.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn end_state_sequence() {
    let s = end_state(0.0); assert!(s.pop < 0.05 && !s.liked && !s.subscribed);
    let s = end_state(0.2); assert!(s.pop > 0.99 && !s.liked);
    let s = end_state(0.4); assert!(s.liked && !s.subscribed);
    let s = end_state(0.6); assert!(s.liked && s.subscribed);
    assert!(end_state(0.7).bell_angle.abs() > 0.05);
    assert_eq!(end_state(0.95).bell_angle, 0.0);
    assert!(end_state(0.9).cursor.is_none());
}
#[test] fn end_card_strings() {
    // main landscape End{t:0.3} → last_strings has "LIKE", "SUBSCRIBE", "Sources"
    // End{t:0.7} → has "SUBSCRIBED"
    // portrait End{t:0.3} → has "SUBSCRIBE" but no "Sources"
}
#[test] fn liked_button_turns_blue() { /* End{t:0.5}: pixel inside the Like button ≈ #2563eb */ }
#[test] fn subscribed_button_turns_grey() { /* End{t:0.7}: pixel inside the Subscribe button ≈ #9ca3af; at t 0.3 it is red #dc2626 */ }
```

- [ ] **Step 2:** Run `cargo test draw`. Expected: FAIL.
- [ ] **Step 3: Implement.** Expose `Painter::button_rects(&self) -> ((f32,f32,f32,f32), (f32,f32,f32,f32))` for the tests: the Like and Subscribe rectangles at full pop, centred in the lower half of the frame. The thumbs-up icon is a simple path (a rounded palm rect plus a thumb) and the bell is a path (a dome, a rim and a clapper circle) rotated about its top. The cursor is an arrow path with a white fill and a dark outline. Main: sources and the estimated note sit above the buttons, the channel name below them (if not empty). Portrait: buttons only, larger.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Render the fixture preview and grab frames at the end-card times.
- [ ] **Step 5:** Commit with the message `feat: like/subscribe/bell end card`.

---

### Task 8: Render options and 12-bar Shorts

**Files:** Modify `src/main.rs`, `src/render.rs`, `src/config.rs`; Test `tests/cli.rs`

**Interfaces:**
- Produces: `render::MusicChoice { Auto, Path(PathBuf), Off }`; `render::RenderOpts { preview: bool, bars: Option<usize>, short_bars: Option<usize>, music: MusicChoice }`; `render::render(week_dir, fmt, opts: &RenderOpts) -> Result<PathBuf>`. CLI: `--bars N`, `--short-bars N`, `--music PATH`, `--no-music` (`--music` and `--no-music` conflict). Default `top_n_short = 12`.

- [ ] **Step 1: Write failing tests** (cli.rs):

```rust
#[test] fn short_bars_option_out_of_range() { /* --short-bars 0 → exit != 0, stderr contains "1–20" */ }
#[test] fn music_flag_overrides() { /* --music <tmp sine.mp3> with no music/ dir → audio stream present */ }
#[test] fn no_music_flag_silent() { /* repo-like cwd with music/t.mp3 + --no-music → no audio stream */ }
#[test] fn music_and_no_music_conflict() { /* both flags → exit != 0 (clap error) */ }
```
Unit test in render.rs: `effective_top_n(cfg, Format::Short, &opts)` returns 12 by default and `opts.short_bars` when set.

- [ ] **Step 2:** Run `cargo test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Add `render::effective_top_n(cfg: &VideoConfig, fmt: Format, opts: &RenderOpts) -> Result<usize>`, which range-checks 1–20 with the message `"--bars/--short-bars must be 1–20"`. Music resolution order: `Off` → none; `Path(p)` → p (must exist); `Auto` → `pick_music` (cfg.music, then ./music).
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit with the message `feat: render options for bars and music, 12-bar shorts`.

---

### Task 9: Bundled CC0 music

**Files:** Create `music/*.mp3` (3–4 tracks), `music/CREDITS.md`; Modify `.gitignore` if needed

- [ ] **Step 1: Choose tracks.** Search OpenGameArt (`https://opengameart.org/art-search-advanced?field_art_type_tid[]=12&field_art_licenses_tid[]=4`, CC0 only) for energetic instrumental tracks of 1–3 minutes (they loop). Open each candidate's page and confirm that "CC0" is the only licence listed. Reject anything that is also tagged CC-BY, GPL or OGA-BY.
- [ ] **Step 2: Download and normalise.** Put each download in its own empty scratch dir, then run `ffmpeg -i in -af loudnorm=I=-16:TP=-1.5 -ac 2 -ar 44100 -b:a 160k music/<slug>.mp3`. Each file should be ≤ 5 MB.
- [ ] **Step 3: Write `music/CREDITS.md`**: for each track, the title, author, page URL, licence (CC0 1.0) and download date.
- [ ] **Step 4: Verify.** Run `cargo test --test cli` (the `no_music_is_silent_ok` test runs in a temp cwd, so it's unaffected). Then run `./target/release/barrace render tests/fixtures/week --preview` from the repo root. Expected: an audio stream is present (`ffprobe`).
- [ ] **Step 5:** Commit with the message `feat: bundle CC0 music tracks`.

---

### Task 10: Sample week refresh, README, timing check

**Files:** Modify `weeks/2026-11-23-sample/data.csv` (raw units), `weeks/2026-11-23-sample/video.toml`; Create `weeks/2026-11-23-sample/events.csv`; Modify `README.md`

- [ ] **Step 1: Data.** Re-scale the sample values to raw people (× 1e6, rounded to an integer) and set `unit_suffix = ""`. Add `events.csv` with facts that can be checked:
  ```csv
  from,to,headline,big,caption
  1999,2002,World population,6 billion,reached in 1999
  2011,2014,World population,7 billion,reached in 2011
  2022,2024,World population,8 billion,reached in 2022
  ```
  Add a `[[pace]]` section to show the feature (1990–2000 at 1.2).
- [ ] **Step 2: README.** Document time formats, pace sections, `number_style`, `bar_colors`, `events.csv`, `theme`, the CLI flags, and the bundled music with its credits.
- [ ] **Step 3: Verify.** Run `cargo test` (all green), then `time ./target/release/barrace render weeks/2026-11-23-sample --format both` and `barrace thumb`. Look at frames: mid-race, a caption, the leader badge, the Short with 12 bars, the end card. Expected: speed in line with the global constraint; record it in the commit message.
- [ ] **Step 6:** Commit with the message `docs: sample week with captions, README for v2`.
