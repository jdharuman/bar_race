use crate::camera::HeightFollow;
use crate::config::{ChannelConfig, Style, VideoConfig};
use crate::walk::WalkPainter;
use crate::data::Dataset;
use crate::draw::{Overlay, Painter};
use crate::encode::{AudioSpec, Encoder};
use crate::layout::Layout;
use crate::scene::{Crossfade, RankTracker};
use crate::timeline::{Format, Phase, PillarTimeline, Timeline};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;

const MUSIC_EXT: [&str; 5] = ["mp3", "m4a", "wav", "ogg", "flac"];

/// Track for this video: `cfg.music` (relative to the week folder), else a stable pick from `music_dir`.
pub fn pick_music(cfg: &VideoConfig, week_dir: &Path, music_dir: &Path) -> Option<PathBuf> {
    if let Some(m) = &cfg.music {
        return Some(week_dir.join(m));
    }
    let mut tracks: Vec<PathBuf> = std::fs::read_dir(music_dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| MUSIC_EXT.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    if tracks.is_empty() {
        return None;
    }
    tracks.sort();
    // FNV-1a over the title: same video always gets the same track
    let h = cfg.title.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    Some(tracks.swap_remove((h % tracks.len() as u64) as usize))
}

/// Render `out/thumb.png` (1280×720) at `year` (default: last year) with a big title overlay.
pub fn thumb(week_dir: &Path, year: Option<f64>) -> Result<PathBuf> {
    let cfg = VideoConfig::load(&week_dir.join("video.toml"))?;
    if cfg.style == Style::Pillars {
        return thumb_pillars(week_dir, &cfg);
    }
    let ds = Dataset::load(week_dir)?;
    let ch = ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let layout = Layout::landscape(1.0, cfg.top_n);
    let year = year.unwrap_or(ds.last);
    let mut tracker = RankTracker::new(ds.items.len(), layout.top_n);
    let mut fs = tracker.step(&ds, year, 1.0 / 60.0);
    for _ in 0..120 {
        fs = tracker.step(&ds, year, 1.0 / 60.0);
    }
    let mut painter = Painter::new(&ds, &cfg, &ch, layout.clone())?;
    let mut pm = Pixmap::new(layout.width, layout.height).unwrap();
    painter.paint_thumb(&mut pm, &fs);
    save_thumb(&pm, week_dir)
}

/// Where the soundtrack comes from.
#[derive(Debug, Clone, Default)]
pub enum MusicChoice {
    /// video.toml `music`, else a track from ./music
    #[default]
    Auto,
    Path(PathBuf),
    Off,
}

#[derive(Debug, Clone, Default)]
pub struct RenderOpts {
    pub preview: bool,
    pub bars: Option<usize>,
    pub short_bars: Option<usize>,
    pub music: MusicChoice,
}

/// Bars shown for this format: CLI override, else video.toml.
pub fn effective_top_n(cfg: &VideoConfig, fmt: Format, opts: &RenderOpts) -> Result<usize> {
    let n = match fmt {
        Format::Main => opts.bars.unwrap_or(cfg.top_n),
        Format::Short => opts.short_bars.unwrap_or(cfg.top_n_short),
    };
    if !(1..=20).contains(&n) {
        bail!("--bars/--short-bars must be 1–20 (got {n})");
    }
    Ok(n)
}

fn resolve_audio(cfg: &VideoConfig, week_dir: &Path, opts: &RenderOpts, duration: f64) -> Result<Option<AudioSpec>> {
    let track = match &opts.music {
        MusicChoice::Off => None,
        MusicChoice::Path(p) => Some(p.clone()),
        MusicChoice::Auto => pick_music(cfg, week_dir, Path::new("music")),
    };
    Ok(match track {
        Some(path) if path.exists() => Some(AudioSpec { path, duration }),
        Some(path) => bail!("music file not found: {}", path.display()),
        None => {
            eprintln!("warning: no music (set music in video.toml or add tracks to ./music); rendering silent");
            None
        }
    })
}

/// Downscale a 1920×1080 frame to out/thumb.png (1280×720).
fn save_thumb(pm: &Pixmap, week_dir: &Path) -> Result<PathBuf> {
    // background is opaque, so premultiplied RGBA == straight RGBA
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec()).unwrap();
    let small = image::imageops::resize(&img, 1280, 720, image::imageops::FilterType::Lanczos3);
    let out_dir = week_dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join("thumb.png");
    small.save(&out)?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}

fn render_pillars(week_dir: &Path, cfg: &VideoConfig, fmt: Format, opts: &RenderOpts) -> Result<PathBuf> {
    let rows = crate::pillars::load(week_dir)?;
    let ch = ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let (scale, fps) = if opts.preview { (0.5, 30) } else { (1.0, 60) };
    let (layout, name, portrait) = match fmt {
        Format::Main => (Layout::landscape(scale, 10), "main.mp4", false),
        Format::Short => (Layout::portrait(scale, 10), "short.mp4", true),
    };
    let tl = PillarTimeline::new(fmt, cfg, rows.len(), fps);
    let out_dir = week_dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join(name);
    let mut walk = WalkPainter::new(&rows, cfg, layout.width, layout.height, portrait)?;
    let mut cards = Painter::new(&crate::pillars::to_dataset(&rows), cfg, &ch, layout.clone())?;
    let values: Vec<f64> = rows.iter().map(|r| r.value).collect();
    let mut follow = HeightFollow::new();
    let mut pm = Pixmap::new(layout.width, layout.height).unwrap();
    let audio = resolve_audio(cfg, week_dir, opts, tl.total_seconds())?;
    let mut enc = Encoder::start(&out, layout.width, layout.height, fps, audio)?;
    let (first, last) = (tl.first as f64, (tl.first + tl.n - 1) as f64);
    let total = tl.frame_count();
    let dt = 1.0 / fps as f64;
    for f in 0..total {
        let phase = tl.phase_at(f);
        let p = match phase {
            Phase::Intro { .. } => first,
            Phase::Race { t } => t,
            Phase::Hold | Phase::End { .. } => last,
        };
        let s = follow.step(&values, p, dt);
        walk.paint(&mut pm, p, s);
        cards.paint_card(&mut pm, &phase);
        enc.write_frame(pm.data())?;
        if total >= 10 && f % (total / 10) == 0 {
            eprintln!("{name}: {}%", f * 100 / total);
        }
    }
    enc.finish()?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}

fn thumb_pillars(week_dir: &Path, cfg: &VideoConfig) -> Result<PathBuf> {
    let rows = crate::pillars::load(week_dir)?;
    let ch = ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let layout = Layout::landscape(1.0, 10);
    let values: Vec<f64> = rows.iter().map(|r| r.value).collect();
    let p = (rows.len() - 1) as f64;
    let mut follow = HeightFollow::new();
    let mut s = 1.0;
    for _ in 0..120 {
        s = follow.step(&values, p, 1.0 / 60.0);
    }
    let mut pm = Pixmap::new(layout.width, layout.height).unwrap();
    // the big title replaces the small walk title
    let no_title = VideoConfig { title: String::new(), ..cfg.clone() };
    WalkPainter::new(&rows, &no_title, layout.width, layout.height, false)?.paint(&mut pm, p, s);
    let mut cards = Painter::new(&crate::pillars::to_dataset(&rows), cfg, &ch, layout)?;
    cards.paint_title_overlay(&mut pm, tiny_skia::Color::WHITE);
    save_thumb(&pm, week_dir)
}

pub fn render(week_dir: &Path, fmt: Format, opts: &RenderOpts) -> Result<PathBuf> {
    let preview = opts.preview;
    let cfg = VideoConfig::load(&week_dir.join("video.toml"))?;
    if !cfg.reviewed {
        bail!("video.toml has reviewed = false: check the data, then set reviewed = true");
    }
    if cfg.style == Style::Pillars {
        return render_pillars(week_dir, &cfg, fmt, opts);
    }
    let ds = Dataset::load(week_dir)?;
    let ch = ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let (scale, fps) = if preview { (0.5, 30) } else { (1.0, 60) };
    let (layout, name) = match fmt {
        Format::Main => (Layout::landscape(scale, effective_top_n(&cfg, fmt, opts)?), "main.mp4"),
        Format::Short => (Layout::portrait(scale, effective_top_n(&cfg, fmt, opts)?), "short.mp4"),
    };
    let tl = Timeline::new(fmt, &cfg, &ds, fps);
    let out_dir = week_dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join(name);
    let (events, ev_errors) = crate::events::parse(week_dir)?;
    if !ev_errors.is_empty() {
        bail!("events.csv has errors:\n  {}", ev_errors.join("\n  "));
    }
    let mut painter = Painter::new(&ds, &cfg, &ch, layout.clone())?;
    painter.set_events(events.clone());
    let mut caption_fade = Crossfade::new(0.3);
    let mut leader_fade = Crossfade::new(0.4);
    let mut tracker = RankTracker::new(ds.items.len(), layout.top_n);
    let mut pm = Pixmap::new(layout.width, layout.height).unwrap();
    let audio = resolve_audio(&cfg, week_dir, opts, tl.total_seconds())?;
    let mut enc = Encoder::start(&out, layout.width, layout.height, fps, audio)?;
    let total = tl.frame_count();
    let dt = 1.0 / fps as f64;
    for f in 0..total {
        let phase = tl.phase_at(f);
        let year = match phase {
            Phase::Intro { .. } => tl.start,
            Phase::Race { t } => t,
            Phase::Hold | Phase::End { .. } => tl.end,
        };
        let fs = tracker.step(&ds, year, dt);
        let ov = Overlay {
            caption: caption_fade.step(crate::events::active(&events, year), dt),
            leader: leader_fade.step(fs.leader, dt),
        };
        painter.paint_with(&mut pm, &phase, &fs, &ov);
        enc.write_frame(pm.data())?;
        if total >= 10 && f % (total / 10) == 0 {
            eprintln!("{name}: {}%", f * 100 / total);
        }
    }
    enc.finish()?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_is_deterministic() {
        let dir = crate::testutil::temp_week("x\n");
        let music = dir.join("music");
        std::fs::create_dir_all(&music).unwrap();
        for f in ["a.mp3", "b.ogg", "c.wav", "notes.txt"] {
            std::fs::write(music.join(f), "").unwrap();
        }
        let cfg = VideoConfig { title: "Largest Economies".into(), ..Default::default() };
        let a = pick_music(&cfg, &dir, &music).unwrap();
        assert_eq!(Some(a.clone()), pick_music(&cfg, &dir, &music));
        assert_ne!(a.extension().unwrap(), "txt");
    }

    #[test]
    fn explicit_music_resolves_against_week() {
        let dir = crate::testutil::temp_week("x\n");
        let cfg = VideoConfig { music: Some("song.mp3".into()), ..Default::default() };
        assert_eq!(pick_music(&cfg, &dir, Path::new("nowhere")), Some(dir.join("song.mp3")));
    }

    #[test]
    fn empty_music_dir_gives_none() {
        let dir = crate::testutil::temp_week("x\n");
        assert_eq!(pick_music(&VideoConfig::default(), &dir, &dir.join("missing")), None);
    }

    #[test]
    fn effective_top_n_defaults_and_overrides() {
        let cfg = VideoConfig::default();
        let opts = RenderOpts::default();
        assert_eq!(effective_top_n(&cfg, Format::Short, &opts).unwrap(), 12);
        assert_eq!(effective_top_n(&cfg, Format::Main, &opts).unwrap(), 10);
        let opts = RenderOpts { bars: Some(15), short_bars: Some(9), ..Default::default() };
        assert_eq!(effective_top_n(&cfg, Format::Short, &opts).unwrap(), 9);
        assert_eq!(effective_top_n(&cfg, Format::Main, &opts).unwrap(), 15);
        let bad = RenderOpts { bars: Some(21), ..Default::default() };
        assert!(effective_top_n(&cfg, Format::Main, &bad).is_err());
    }
}
