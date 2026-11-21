use crate::config::{ChannelConfig, VideoConfig};
use crate::data::Dataset;
use crate::draw::{Overlay, Painter};
use crate::encode::{AudioSpec, Encoder};
use crate::layout::Layout;
use crate::scene::{Crossfade, RankTracker};
use crate::timeline::{Format, Phase, Timeline};
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
    // background is opaque, so premultiplied RGBA == straight RGBA
    let img = image::RgbaImage::from_raw(layout.width, layout.height, pm.data().to_vec()).unwrap();
    let small = image::imageops::resize(&img, 1280, 720, image::imageops::FilterType::Lanczos3);
    let out_dir = week_dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join("thumb.png");
    small.save(&out)?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}

pub fn render(week_dir: &Path, fmt: Format, preview: bool) -> Result<PathBuf> {
    let cfg = VideoConfig::load(&week_dir.join("video.toml"))?;
    if !cfg.reviewed {
        bail!("video.toml has reviewed = false: check data.csv, then set reviewed = true");
    }
    let ds = Dataset::load(week_dir)?;
    let ch = ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let (scale, fps) = if preview { (0.5, 30) } else { (1.0, 60) };
    let (layout, name) = match fmt {
        Format::Main => (Layout::landscape(scale, cfg.top_n), "main.mp4"),
        Format::Short => (Layout::portrait(scale, cfg.top_n_short), "short.mp4"),
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
    let audio = match pick_music(&cfg, week_dir, Path::new("music")) {
        Some(path) if path.exists() => Some(AudioSpec { path, duration: tl.total_seconds() }),
        Some(path) => bail!("music file not found: {}", path.display()),
        None => {
            eprintln!("warning: no music (set music in video.toml or add tracks to ./music); rendering silent");
            None
        }
    };
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
}
