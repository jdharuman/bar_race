use crate::config::{ChannelConfig, VideoConfig};
use crate::data::Dataset;
use crate::fmt::format_value;
use crate::layout::Layout;
use crate::scene::{BarState, FrameState};
use crate::text::{Align, Text};
use crate::timeline::Phase;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;
use tiny_skia::{Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

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
    images: Vec<Option<Pixmap>>,
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
        let d = layout.bar_height().round().max(1.0) as u32;
        let images = ds
            .items
            .iter()
            .map(|it| {
                let path = it.image.as_ref()?;
                match load_circle_image(path, d) {
                    Ok(pm) => Some(pm),
                    Err(e) => {
                        eprintln!("warning: image {}: {e:#}; using initials", path.display());
                        None
                    }
                }
            })
            .collect();
        Ok(Painter {
            ds: ds.clone(),
            cfg: cfg.clone(),
            ch: ch.clone(),
            layout,
            text: Text::new(),
            item_colors,
            images,
            bg: hex(&ch.background),
            fg: hex(&ch.text),
            last_strings: Vec::new(),
        })
    }

    #[cfg(test)]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Thumbnail: race frame without the small title block, oversized title in the header area.
    pub fn paint_thumb(&mut self, pm: &mut Pixmap, fs: &FrameState) {
        self.last_strings.clear();
        pm.fill(self.bg);
        self.paint_race_inner(pm, fs, false);
        let l = self.layout.clone();
        let title = self.cfg.title.clone();
        let size = self.fit_size(&title, l.title_size * 2.2, true, l.width as f32 * 0.92);
        self.txt(pm, &title, l.width as f32 / 2.0, l.chart_top * 0.45, size, true, self.fg, Align::Center);
    }

    /// Largest size ≤ `size` at which `s` fits in `max_w`.
    fn fit_size(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> f32 {
        let w = self.text.measure(s, size, bold);
        if w <= max_w { size } else { (size * max_w / w).floor() }
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
        match *phase {
            Phase::Intro { t } => self.paint_intro(pm, t),
            Phase::End { t } => self.paint_end(pm, t),
            Phase::Race { .. } | Phase::Hold => {}
        }
    }

    fn dim(&self, pm: &mut Pixmap, a: f32) {
        let mut paint = Paint::default();
        paint.set_color(with_alpha(self.bg, a));
        let r = Rect::from_xywh(0.0, 0.0, pm.width() as f32, pm.height() as f32).unwrap();
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }

    fn paint_intro(&mut self, pm: &mut Pixmap, t: f64) {
        let t = t as f32;
        let fade_out = ((1.0 - t) / 0.3).min(1.0); // overlay lifts over the last 30%
        self.dim(pm, 0.75 * fade_out);
        let a = (t / 0.3).min(1.0) * fade_out;
        let l = self.layout.clone();
        let (cx, cy) = (l.width as f32 / 2.0, l.height as f32 * 0.45);
        let (title, sub) = (self.cfg.title.clone(), self.cfg.subtitle.clone());
        let max_w = l.width as f32 * 0.9;
        let ts = self.fit_size(&title, l.title_size * 2.0, true, max_w);
        self.txt(pm, &title, cx, cy, ts, true, with_alpha(self.fg, a), Align::Center);
        if !sub.is_empty() {
            let y = cy + l.title_size * 1.6;
            let ss = self.fit_size(&sub, l.subtitle_size * 1.4, false, max_w);
            self.txt(pm, &sub, cx, y, ss, false, with_alpha(self.fg, 0.8 * a), Align::Center);
        }
    }

    fn paint_end(&mut self, pm: &mut Pixmap, t: f64) {
        let a = (t as f32 / 0.2).min(1.0);
        self.dim(pm, 0.88 * a);
        let l = self.layout.clone();
        let fg = with_alpha(self.fg, a);
        let cx = l.width as f32 / 2.0;
        let line = l.label_size * 1.6;
        let mut y = l.height as f32 * 0.3;
        self.txt(pm, "Sources", cx, y, l.title_size, true, fg, Align::Center);
        y += line * 1.5;
        let mut domains: Vec<String> = Vec::new();
        for src in self.ds.items.iter().flat_map(|i| &i.sources) {
            let d = domain(src);
            if !d.is_empty() && !domains.contains(&d) {
                domains.push(d);
            }
        }
        for d in domains.iter().take(5) {
            self.txt(pm, d, cx, y, l.label_size, false, with_alpha(fg, 0.85), Align::Center);
            y += line;
        }
        let share = self.ds.estimated_share();
        if share > 0.0 {
            y += line * 0.5;
            let note = format!("* estimated values: {:.0}%", share * 100.0);
            self.txt(pm, &note, cx, y, l.label_size, false, with_alpha(fg, 0.7), Align::Center);
        }
        let name = self.ch.name.clone();
        self.txt(pm, &name, cx, l.height as f32 * 0.82, l.title_size * 0.8, true, fg, Align::Center);
    }

    /// Centre of the image/initials circle that sits just right of a bar's end.
    pub fn bar_tip(&self, b: &BarState, axis_max: f64) -> (f32, f32) {
        let l = &self.layout;
        let bh = l.bar_height();
        let bw = bar_width(l, b.value, axis_max);
        (l.chart_left + bw + l.label_size * 0.4 + bh / 2.0, l.slot_center_y(b.slot))
    }

    fn paint_axis(&mut self, pm: &mut Pixmap, axis_max: f64) {
        let l = self.layout.clone();
        let chart_w = l.chart_right - l.chart_left;
        let mut paint = Paint::default();
        paint.set_color(with_alpha(self.fg, 0.10));
        let tick_size = l.label_size * 0.75;
        for v in nice_ticks(axis_max) {
            let x = l.chart_left + (v / axis_max) as f32 * chart_w;
            if let Some(r) = Rect::from_xywh(x, l.chart_top - tick_size * 0.4, (l.height as f32 / 1080.0).max(1.0), l.chart_bottom - l.chart_top) {
                pm.fill_rect(r, &paint, Transform::identity(), None);
            }
            let label = format_value(v, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style);
            let y = l.chart_top - tick_size * 1.1;
            self.txt(pm, &label, x, y, tick_size, false, with_alpha(self.fg, 0.45), Align::Center);
        }
    }

    fn paint_race(&mut self, pm: &mut Pixmap, fs: &FrameState) {
        self.paint_race_inner(pm, fs, true);
    }

    fn paint_race_inner(&mut self, pm: &mut Pixmap, fs: &FrameState, with_title: bool) {
        let l = self.layout.clone();
        // title block
        let title = self.cfg.title.clone();
        let subtitle = self.cfg.subtitle.clone();
        let max_w = l.width as f32 - 2.0 * l.title_pos.0;
        let ts = self.fit_size(&title, l.title_size, true, max_w);
        if with_title {
            self.txt(pm, &title, l.title_pos.0, l.title_pos.1, ts, true, self.fg, Align::Left);
        }
        if with_title && !subtitle.is_empty() {
            let ss = self.fit_size(&subtitle, l.subtitle_size, false, max_w);
            let y = l.title_pos.1 + l.title_size * 0.9 + l.subtitle_size * 0.6;
            self.txt(pm, &subtitle, l.title_pos.0, y, ss, false, with_alpha(self.fg, 0.7), Align::Left);
        }
        // year counter
        let year = crate::fmt::format_time(fs.t, self.ds.granularity);
        let align = if l.year_centered { Align::Center } else { Align::Right };
        self.txt(pm, &year, l.year_pos.0, l.year_pos.1, l.year_size, true, with_alpha(self.fg, 0.25), align);
        self.paint_axis(pm, fs.axis_max);
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
        let l = self.layout.clone();
        let cy = l.slot_center_y(b.slot);
        let bh = l.bar_height();
        let bw = bar_width(&l, b.value, axis_max);
        let (x0, label_size) = (l.chart_left, l.label_size);
        let color = with_alpha(self.item_colors[b.idx], b.alpha);
        if bw >= 1.0 {
            let r = (bh / 2.0).min(bw);
            let path = rounded_bar(x0, cy - bh / 2.0, bw, bh, r);
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        // image or initials circle just past the bar end
        let (icx, _) = self.bar_tip(b, axis_max);
        let circle = PathBuilder::from_circle(icx, cy, bh / 2.0).unwrap();
        match &self.images[b.idx] {
            Some(img) => {
                let paint = PixmapPaint { opacity: b.alpha, ..Default::default() };
                let (ix, iy) = ((icx - bh / 2.0).round() as i32, (cy - bh / 2.0).round() as i32);
                pm.draw_pixmap(ix, iy, img.as_ref(), &paint, Transform::identity(), None);
            }
            None => {
                let mut paint = Paint::default();
                let c = self.item_colors[b.idx];
                paint.set_color(Color::from_rgba(c.red() * 0.55, c.green() * 0.55, c.blue() * 0.55, b.alpha).unwrap());
                paint.anti_alias = true;
                pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
                let ini = initials(&self.ds.items[b.idx].name);
                self.txt(pm, &ini, icx, cy, bh * 0.42, true, with_alpha(self.fg, b.alpha), Align::Center);
            }
        }
        let pad = label_size * 0.5;
        let name = self.ds.items[b.idx].name.clone();
        let name_w = self.text.measure(&name, label_size, true);
        let fg = with_alpha(self.fg, b.alpha);
        let mut after = icx + bh / 2.0 + pad;
        if name_w + 2.0 * pad <= bw {
            self.txt(pm, &name, x0 + pad, cy, label_size, true, fg, Align::Left);
        } else {
            self.txt(pm, &name, after, cy, label_size, true, fg, Align::Left);
            after += name_w + pad;
        }
        let mut value = format_value(b.value, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style);
        if b.estimated {
            value.push('*');
        }
        self.txt(pm, &value, after, cy, label_size, false, with_alpha(fg, 0.85), Align::Left);
    }
}

fn bar_width(l: &Layout, value: f64, axis_max: f64) -> f32 {
    let chart_w = l.chart_right - l.chart_left;
    ((value.max(0.0) / axis_max.max(1e-9)) as f32 * chart_w).clamp(0.0, chart_w)
}

/// Up to two initials from the first letters of the name's words.
fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect()
}

/// "https://www.worldbank.org/x" → "worldbank.org"
fn domain(url: &str) -> String {
    let s = url.split("://").nth(1).unwrap_or(url);
    let host = s.split('/').next().unwrap_or("");
    host.trim_start_matches("www.").to_string()
}

/// Gridline values 0, step, 2·step… ≤ max, with step a 1/2/5×10^k giving about five ticks.
pub fn nice_ticks(max: f64) -> Vec<f64> {
    if !(max > 0.0) {
        return vec![0.0];
    }
    let raw = max / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag * if norm <= 1.0 { 1.0 } else if norm <= 2.0 { 2.0 } else if norm <= 5.0 { 5.0 } else { 10.0 };
    (0..).map(|i| i as f64 * step).take_while(|v| *v <= max).collect()
}

/// Load an image, crop to a centred square, scale to d×d and cut to a circle (premultiplied).
fn load_circle_image(path: &Path, d: u32) -> Result<Pixmap> {
    let img = image::open(path)?.resize_to_fill(d, d, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut square = Pixmap::new(d, d).unwrap();
    for (dst, src) in square.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        let pre = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *dst = tiny_skia::PremultipliedColorU8::from_rgba(pre(r), pre(g), pre(b), a).unwrap();
    }
    let r = d as f32 / 2.0;
    let mut mask = Mask::new(d, d).unwrap();
    mask.fill_path(&PathBuilder::from_circle(r, r, r).unwrap(), FillRule::Winding, true, Transform::identity());
    let mut out = Pixmap::new(d, d).unwrap();
    out.draw_pixmap(0, 0, square.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
    Ok(out)
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
        p.paint(&mut pm, &Phase::Race { t: 2001.0 }, &fs);
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
        p.paint(&mut pm, &Phase::Race { t: 2002.0 }, &fs);
        assert!(p.last_strings.iter().any(|s| s == "Delta"));
    }

    #[test]
    fn negative_value_zero_length_bar() {
        let mk = |n: &str, cat: &str, v: f64| Item {
            name: n.into(),
            category: cat.into(),
            image: None,
            points: vec![Point { t: 2000 as f64, value: v, estimated: false }, Point { t: 2001 as f64, value: v, estimated: false }],
            sources: vec![],
        };
        let ds = Dataset::from_items(vec![mk("Pos", "P", 100.0), mk("Neg", "N", -5.0)]);
        let fs = settle(&ds, 10, 2000.5);
        let mut p = painter(&ds);
        let l = p.layout().clone();
        let mut pm = Pixmap::new(l.width, l.height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2000.5 }, &fs);
        let y = l.slot_center_y(1.0) as u32;
        // category N is palette[1]; no bar pixels of that colour in the Neg row
        assert_eq!(count_color_in_row(&pm, y, [0xf4, 0x72, 0xb6]), 0);
        assert!(p.last_strings.iter().any(|s| s == "-5.0"));
    }

    fn one_item(image: Option<std::path::PathBuf>) -> Dataset {
        Dataset::from_items(vec![Item {
            name: "Red Land".into(),
            category: "X".into(),
            image,
            points: vec![Point { t: 2000 as f64, value: 10.0, estimated: false }, Point { t: 2001 as f64, value: 10.0, estimated: false }],
            sources: vec!["https://www.worldbank.org/x".into()],
        }])
    }

    #[test]
    fn image_drawn_at_bar_tip() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("red.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
        let ds = one_item(Some(png));
        let fs = settle(&ds, 10, 2000.5);
        let mut p = painter(&ds);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2000.5 }, &fs);
        let (cx, cy) = p.bar_tip(&fs.bars[0], fs.axis_max);
        let px = pm.pixel(cx as u32, cy as u32).unwrap();
        assert!(px.red() > 230 && px.green() < 30 && px.blue() < 30, "{px:?}");
    }

    #[test]
    fn initials_fallback() {
        let ds = one_item(None);
        let fs = settle(&ds, 10, 2000.5);
        let mut p = painter(&ds);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2000.5 }, &fs);
        assert!(p.last_strings.iter().any(|s| s == "RL"), "{:?}", p.last_strings);
    }

    #[test]
    fn axis_ticks_are_nice() {
        assert_eq!(nice_ticks(35.0), vec![0.0, 10.0, 20.0, 30.0]);
        assert_eq!(nice_ticks(2.4e12), vec![0.0, 5e11, 1e12, 1.5e12, 2e12]);
        assert_eq!(nice_ticks(0.9), vec![0.0, 0.2, 0.4, 0.6000000000000001, 0.8]);
    }

    #[test]
    fn intro_card_shows_title() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2000.0);
        let cfg = VideoConfig { title: "Big Title".into(), ..Default::default() };
        let mut p = Painter::new(&ds, &cfg, &ChannelConfig::default(), Layout::landscape(0.5, 10)).unwrap();
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Intro { t: 0.5 }, &fs);
        let (w, h) = (pm.width(), pm.height());
        let bright = (h / 3..2 * h / 3)
            .flat_map(|y| (w / 4..3 * w / 4).map(move |x| (x, y)))
            .filter(|&(x, y)| pm.pixel(x, y).unwrap().red() > 200 && pm.pixel(x, y).unwrap().blue() > 200)
            .count();
        assert!(bright >= 500, "{bright}");
    }

    #[test]
    fn end_card_estimated_note() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2003.0);
        let mut p = painter(&ds);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::End { t: 0.5 }, &fs);
        assert!(p.last_strings.iter().any(|s| s == "* estimated values: 8%"), "{:?}", p.last_strings);
        assert!(p.last_strings.iter().any(|s| s == "example.org"), "{:?}", p.last_strings);
    }

    #[test]
    fn long_title_fits_portrait_width() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 8, 2001.0);
        let cfg = VideoConfig { title: "The Largest Economies in the Entire World by Nominal GDP".into(), ..Default::default() };
        let mut p = Painter::new(&ds, &cfg, &ChannelConfig::default(), Layout::portrait(0.5, 8)).unwrap();
        let (w, h) = (p.layout().width, p.layout().height);
        let mut pm = Pixmap::new(w, h).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2001.0 }, &fs);
        let edge = w - w * 4 / 100;
        let lit = (0..h * 15 / 100)
            .flat_map(|y| (edge..w).map(move |x| (x, y)))
            .filter(|&(x, y)| pm.pixel(x, y).unwrap().red() > 150)
            .count();
        assert_eq!(lit, 0, "title spills past right margin");
    }

    #[test]
    fn thumb_draws_title_once() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2003.0);
        let cfg = VideoConfig { title: "Big Title".into(), ..Default::default() };
        let mut p = Painter::new(&ds, &cfg, &ChannelConfig::default(), Layout::landscape(0.5, 10)).unwrap();
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint_thumb(&mut pm, &fs);
        assert_eq!(p.last_strings.iter().filter(|s| *s == "Big Title").count(), 1, "{:?}", p.last_strings);
    }

    #[test]
    fn circle_image_has_transparent_corners() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("red.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
        let pm = load_circle_image(&png, 40).unwrap();
        assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
        assert_eq!(pm.pixel(20, 20).unwrap().alpha(), 255);
    }
}
