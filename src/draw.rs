use crate::config::{ChannelConfig, VideoConfig};
use crate::data::Dataset;
use crate::fmt::format_value;
use crate::layout::Layout;
use crate::scene::{BarState, FrameState};
use crate::text::{Align, Text};
use crate::timeline::Phase;
use anyhow::Result;
use std::collections::BTreeMap;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

const PALETTE: [&str; 12] = [
    "#38bdf8", "#f472b6", "#facc15", "#4ade80", "#fb923c", "#a78bfa", "#f87171", "#2dd4bf", "#e879f9", "#a3e635",
    "#60a5fa", "#fbbf24",
];

pub fn hex(s: &str) -> Color {
    let s = s.trim().trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(s.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    Color::from_rgba8(c(0), c(2), c(4), 255)
}

fn with_alpha(mut c: Color, a: f32) -> Color {
    c.set_alpha((c.alpha() * a).clamp(0.0, 1.0));
    c
}

pub struct Painter {
    ds: Dataset,
    cfg: VideoConfig,
    ch: ChannelConfig,
    layout: Layout,
    text: Text,
    item_colors: Vec<Color>,
    bg: Color,
    fg: Color,
    /// Strings drawn by the last `paint` call (used by tests and the end card).
    pub last_strings: Vec<String>,
}

impl Painter {
    pub fn new(ds: &Dataset, cfg: &VideoConfig, ch: &ChannelConfig, layout: Layout) -> Result<Painter> {
        let mut cat_colors: BTreeMap<&str, Color> = BTreeMap::new();
        let mut next = 0;
        for it in &ds.items {
            if !cat_colors.contains_key(it.category.as_str()) {
                let c = match cfg.colors.get(&it.category) {
                    Some(h) => hex(h),
                    None => {
                        next += 1;
                        hex(PALETTE[(next - 1) % PALETTE.len()])
                    }
                };
                cat_colors.insert(&it.category, c);
            }
        }
        let item_colors = ds.items.iter().map(|it| cat_colors[it.category.as_str()]).collect();
        Ok(Painter {
            ds: ds.clone(),
            cfg: cfg.clone(),
            ch: ch.clone(),
            layout,
            text: Text::new(),
            item_colors,
            bg: hex(&ch.background),
            fg: hex(&ch.text),
            last_strings: Vec::new(),
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    #[allow(clippy::too_many_arguments)]
    fn txt(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align) {
        self.text.draw(pm, s, x, y, size, bold, color, align);
        self.last_strings.push(s.to_string());
    }

    pub fn paint(&mut self, pm: &mut Pixmap, phase: &Phase, fs: &FrameState) {
        self.last_strings.clear();
        pm.fill(self.bg);
        self.paint_race(pm, fs);
        let _ = phase; // cards: Task 6
    }

    fn paint_race(&mut self, pm: &mut Pixmap, fs: &FrameState) {
        let l = self.layout.clone();
        // title block
        let title = self.cfg.title.clone();
        let subtitle = self.cfg.subtitle.clone();
        self.txt(pm, &title, l.title_pos.0, l.title_pos.1, l.title_size, true, self.fg, Align::Left);
        if !subtitle.is_empty() {
            let y = l.title_pos.1 + l.title_size * 0.9 + l.subtitle_size * 0.6;
            self.txt(pm, &subtitle, l.title_pos.0, y, l.subtitle_size, false, with_alpha(self.fg, 0.7), Align::Left);
        }
        // year counter
        let year = format!("{}", fs.year.floor() as i64);
        let align = if l.year_centered { Align::Center } else { Align::Right };
        self.txt(pm, &year, l.year_pos.0, l.year_pos.1, l.year_size, true, with_alpha(self.fg, 0.25), align);
        // bars, bottom-most first so overtaking bars draw on top
        for b in fs.bars.iter().rev() {
            self.paint_bar(pm, b, fs.axis_max);
        }
        // watermark
        let name = self.ch.name.clone();
        let (wx, wy) = l.watermark_pos;
        self.txt(pm, &name, wx, wy, l.watermark_size, true, with_alpha(self.fg, 0.4), Align::Right);
    }

    fn paint_bar(&mut self, pm: &mut Pixmap, b: &BarState, axis_max: f64) {
        let l = &self.layout;
        let cy = l.slot_center_y(b.slot);
        let bh = l.bar_height();
        let chart_w = l.chart_right - l.chart_left;
        let bw = ((b.value.max(0.0) / axis_max.max(1e-9)) as f32 * chart_w).clamp(0.0, chart_w);
        let (x0, label_size) = (l.chart_left, l.label_size);
        if bw >= 1.0 {
            let r = (bh / 2.0).min(bw);
            let path = rounded_bar(x0, cy - bh / 2.0, bw, bh, r);
            let mut paint = Paint::default();
            paint.set_color(with_alpha(self.item_colors[b.idx], b.alpha));
            paint.anti_alias = true;
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        let pad = label_size * 0.5;
        let name = self.ds.items[b.idx].name.clone();
        let name_w = self.text.measure(&name, label_size, true);
        let fg = with_alpha(self.fg, b.alpha);
        let mut after = x0 + bw + pad;
        if name_w + 2.0 * pad <= bw {
            self.txt(pm, &name, x0 + pad, cy, label_size, true, fg, Align::Left);
        } else {
            self.txt(pm, &name, after, cy, label_size, true, fg, Align::Left);
            after += name_w + pad;
        }
        let mut value = format_value(b.value, &self.cfg.unit_prefix, &self.cfg.unit_suffix);
        if b.estimated {
            value.push('*');
        }
        self.txt(pm, &value, after, cy, label_size, false, with_alpha(fg, 0.85), Align::Left);
    }
}

/// Bar from (x, y) of width w and height h whose right end is rounded with radius r.
fn rounded_bar(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    const K: f32 = 0.552_284_8;
    let right = x + w;
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(right - r, y);
    pb.cubic_to(right - r + r * K, y, right, y + r - r * K, right, y + r);
    pb.line_to(right, y + h - r);
    pb.cubic_to(right, y + h - r + r * K, right - r + r * K, y + h, right - r, y + h);
    pb.line_to(x, y + h);
    pb.close();
    pb.finish().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelConfig, VideoConfig};
    use crate::data::{Dataset, Item, Point};
    use crate::layout::Layout;
    use crate::scene::RankTracker;
    use crate::timeline::Phase;
    use std::path::Path;
    use tiny_skia::Pixmap;

    fn settle(ds: &Dataset, top_n: usize, year: f64) -> crate::scene::FrameState {
        let mut t = RankTracker::new(ds.items.len(), top_n);
        let mut fs = t.step(ds, year, 1.0 / 60.0);
        for _ in 0..120 {
            fs = t.step(ds, year, 1.0 / 60.0);
        }
        fs
    }

    fn painter(ds: &Dataset) -> Painter {
        Painter::new(ds, &VideoConfig::default(), &ChannelConfig::default(), Layout::landscape(0.5, 10)).unwrap()
    }

    fn count_color_in_row(pm: &Pixmap, y: u32, c: [u8; 3]) -> usize {
        (0..pm.width())
            .filter(|&x| {
                let p = pm.pixel(x, y).unwrap();
                (p.red() as i32 - c[0] as i32).abs() <= 8
                    && (p.green() as i32 - c[1] as i32).abs() <= 8
                    && (p.blue() as i32 - c[2] as i32).abs() <= 8
            })
            .count()
    }

    #[test]
    fn paints_bar_in_category_color() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2001.0);
        let mut p = painter(&ds);
        let l = p.layout().clone();
        let mut pm = Pixmap::new(l.width, l.height).unwrap();
        p.paint(&mut pm, &Phase::Race { year: 2001.0 }, &fs);
        // top bar at 2001 is Beta (22), category Y = second category seen → palette[1] #f472b6
        let y = l.slot_center_y(0.0) as u32;
        assert!(count_color_in_row(&pm, y, [0xf4, 0x72, 0xb6]) > 50);
    }

    #[test]
    fn fewer_items_than_top_n() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2002.0);
        let mut p = painter(&ds);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { year: 2002.0 }, &fs);
        assert!(p.last_strings.iter().any(|s| s == "Delta"));
    }

    #[test]
    fn negative_value_zero_length_bar() {
        let mk = |n: &str, cat: &str, v: f64| Item {
            name: n.into(),
            category: cat.into(),
            image: None,
            points: vec![Point { year: 2000, value: v, estimated: false }, Point { year: 2001, value: v, estimated: false }],
            sources: vec![],
        };
        let ds = Dataset::from_items(vec![mk("Pos", "P", 100.0), mk("Neg", "N", -5.0)]);
        let fs = settle(&ds, 10, 2000.5);
        let mut p = painter(&ds);
        let l = p.layout().clone();
        let mut pm = Pixmap::new(l.width, l.height).unwrap();
        p.paint(&mut pm, &Phase::Race { year: 2000.5 }, &fs);
        let y = l.slot_center_y(1.0) as u32;
        // category N is palette[1]; no bar pixels of that colour in the Neg row
        assert_eq!(count_color_in_row(&pm, y, [0xf4, 0x72, 0xb6]), 0);
        assert!(p.last_strings.iter().any(|s| s == "-5.0"));
    }
}
