use crate::config::{ChannelConfig, VideoConfig};
use crate::data::Dataset;
use crate::draw::Painter;
use crate::encode::Encoder;
use crate::layout::Layout;
use crate::scene::RankTracker;
use crate::timeline::{Format, Phase, Timeline};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;

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
        Format::Short => bail!("short format not implemented yet"),
    };
    let tl = Timeline::new(fmt, &cfg, &ds, fps);
    let out_dir = week_dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join(name);
    let mut painter = Painter::new(&ds, &cfg, &ch, layout.clone())?;
    let mut tracker = RankTracker::new(ds.items.len(), layout.top_n);
    let mut pm = Pixmap::new(layout.width, layout.height).unwrap();
    let mut enc = Encoder::start(&out, layout.width, layout.height, fps, None)?;
    let total = tl.frame_count();
    let dt = 1.0 / fps as f64;
    for f in 0..total {
        let phase = tl.phase_at(f);
        let year = match phase {
            Phase::Intro { .. } => tl.start_year as f64,
            Phase::Race { year } => year,
            Phase::Hold | Phase::End { .. } => tl.end_year as f64,
        };
        let fs = tracker.step(&ds, year, dt);
        painter.paint(&mut pm, &phase, &fs);
        enc.write_frame(pm.data())?;
        if total >= 10 && f % (total / 10) == 0 {
            eprintln!("{name}: {}%", f * 100 / total);
        }
    }
    enc.finish()?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}
