//! Animated line chart (`style = "lines"`): a few series drawn left to right over time,
//! e.g. "value of $1,000 invested" in 2–3 stocks. Reuses data.csv, the Timeline,
//! events and the intro/end cards; this module draws the chart itself.
use crate::config::{ChannelConfig, Theme, VideoConfig};
use crate::data::{Dataset, Item};
use crate::draw::{hex, initials, load_circle_image, nice_ticks, ring, with_alpha};
use crate::fmt::{format_tick, format_time, format_value};
use crate::layout::Layout;
use crate::text::{Align, Text};
use anyhow::Result;
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

/// The y-axis top sits this far above the highest value shown so far.
const Y_HEADROOM: f64 = 1.12;
/// Seconds for the y-axis to catch up with a new high.
const Y_TAU: f64 = 0.35;
/// Distinct line colours (light theme / dark theme), picked for contrast, not from logos:
/// two blue logos would otherwise give two blue lines.
const LIGHT: [&str; 6] = ["#2563eb", "#e11d48", "#16a34a", "#d97706", "#7c3aed", "#0891b2"];
const DARK: [&str; 6] = ["#60a5fa", "#fb7185", "#4ade80", "#fbbf24", "#a78bfa", "#22d3ee"];

/// Value of `item` at `t` (linear between points); None outside its data range.
/// Unlike the bar race there is no ramp: a line simply starts and stops.
pub fn series_at(item: &Item, t: f64) -> Option<f64> {
    let pts = &item.points;
    let (first, last) = (pts.first()?, pts.last()?);
    if !t.is_finite() || t < first.t - 1e-9 || t > last.t + 1e-9 {
        return None;
    }
    let i = pts.partition_point(|p| p.t <= t);
    if i == 0 {
        return Some(first.value);
    }
    if i == pts.len() {
        return Some(last.value);
    }
    let (a, b) = (&pts[i - 1], &pts[i]);
    Some(a.value + (b.value - a.value) * (t - a.t) / (b.t - a.t))
}

/// Per-item multiplier: `rebase / value at start` (each line starts at `rebase`), or 1.
/// NaN for an item with no positive value at `start` (validate rejects that).
pub fn scales(ds: &Dataset, start: f64, rebase: Option<f64>) -> Vec<f64> {
    ds.items
        .iter()
        .map(|it| match rebase {
            Some(r) => series_at(it, start).filter(|v| *v > 0.0).map_or(f64::NAN, |v| r / v),
            None => 1.0,
        })
        .collect()
}

/// `{start}` in the title/subtitle → the first month this video shows (the Short may
/// start later than the data, and "invested in Jan 1999" must match what is drawn).
pub fn fill_start(cfg: &VideoConfig, start: f64, g: crate::data::Granularity) -> VideoConfig {
    let s = format_time(start, g);
    VideoConfig { title: cfg.title.replace("{start}", &s), subtitle: cfg.subtitle.replace("{start}", &s), ..cfg.clone() }
}

/// Layout for the line chart: wider plot, room on the right for line labels.
pub fn layout(scale: f32, portrait: bool) -> Layout {
    let mut l = if portrait { Layout::portrait(scale, 10) } else { Layout::landscape(scale, 10) };
    let (w, h) = (l.width as f32, l.height as f32);
    if portrait {
        (l.chart_left, l.chart_top, l.chart_right, l.chart_bottom) = (0.13 * w, 0.20 * h, 0.64 * w, 0.76 * h);
        l.caption_at = Some((0.06 * w, 0.795 * h, 0.88 * w, 0.075 * h));
    } else {
        (l.chart_left, l.chart_top, l.chart_right, l.chart_bottom) = (0.08 * w, 0.20 * h, 0.75 * w, 0.86 * h);
        l.caption_at = Some((0.10 * w, 0.215 * h, 0.38 * w, 0.16 * h));
    }
    l
}

/// What one frame shows.
#[derive(Debug, Clone)]
pub struct LineFrame {
    /// Current time (the line heads).
    pub t: f64,
    pub x_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

/// Axis state across frames.
pub struct LineTracker {
    start: f64,
    min_span: f64,
    scales: Vec<f64>,
    y_max: f64,
}

impl LineTracker {
    pub fn new(ds: &Dataset, start: f64, end: f64, rebase: Option<f64>) -> LineTracker {
        let span = (end - start).max(1e-6);
        LineTracker { start, min_span: (span * 0.12).clamp(0.5, 2.0).min(span), scales: scales(ds, start, rebase), y_max: 0.0 }
    }

    pub fn scales(&self) -> &[f64] {
        &self.scales
    }

    /// Highest scaled value of any line within [start, t].
    pub fn visible_max(&self, ds: &Dataset, t: f64) -> f64 {
        let mut m: f64 = 0.0;
        for (it, k) in ds.items.iter().zip(&self.scales) {
            if !k.is_finite() {
                continue;
            }
            for p in it.points.iter().filter(|p| p.t > self.start && p.t < t) {
                m = m.max(p.value * k);
            }
            for tt in [self.start, t] {
                if let Some(v) = series_at(it, tt.min(it.points.last().map_or(tt, |p| p.t))) {
                    m = m.max(v * k);
                }
            }
        }
        m
    }

    pub fn step(&mut self, ds: &Dataset, t: f64, dt: f64) -> LineFrame {
        let t = t.max(self.start);
        let vmax = self.visible_max(ds, t).max(1e-9);
        let target = vmax * Y_HEADROOM;
        self.y_max = if self.y_max <= 0.0 { target } else { self.y_max + (target - self.y_max) * (1.0 - (-dt / Y_TAU).exp()) };
        self.y_max = self.y_max.max(vmax * 1.02); // a line never leaves the chart
        LineFrame { t, x_min: self.start, x_max: t.max(self.start + self.min_span), y_max: self.y_max }
    }
}

/// Move label centres apart so neighbours are at least `gap` apart, keeping them in [lo, hi].
/// Order is preserved (sorted by the wanted position).
pub fn spread(wanted: &[f32], gap: f32, lo: f32, hi: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by(|&a, &b| wanted[a].total_cmp(&wanted[b]));
    let mut ys: Vec<f32> = order.iter().map(|&i| wanted[i].clamp(lo, hi)).collect();
    for k in 1..ys.len() {
        ys[k] = ys[k].max(ys[k - 1] + gap);
    }
    if let Some(&last) = ys.last() {
        if last > hi {
            let n = ys.len();
            ys[n - 1] = hi;
            for k in (0..n - 1).rev() {
                ys[k] = ys[k].min(ys[k + 1] - gap);
            }
        }
    }
    let mut out = vec![0.0; wanted.len()];
    for (k, &i) in order.iter().enumerate() {
        out[i] = ys[k];
    }
    out
}

/// Year tick step giving at most `max_ticks` ticks over `span` years.
fn year_step(span: f64, max_ticks: f64) -> f64 {
    [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0].into_iter().find(|s| span / s <= max_ticks).unwrap_or(100.0)
}

pub struct LinePainter {
    ds: Dataset,
    cfg: VideoConfig,
    ch: ChannelConfig,
    l: Layout,
    text: Text,
    colors: Vec<Color>,
    heads: Vec<Option<Pixmap>>,
    head_d: f32,
    scales: Vec<f64>,
    bg: Color,
    fg: Color,
    grid_a: f32,
    /// Strings drawn by the last `paint` (tests).
    pub last_strings: Vec<String>,
}

impl LinePainter {
    pub fn new(ds: &Dataset, cfg: &VideoConfig, ch: &ChannelConfig, l: Layout, scales: Vec<f64>) -> Result<LinePainter> {
        let palette = if ch.theme == Theme::Dark { DARK } else { LIGHT };
        let colors = ds
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let custom = cfg.colors.get(&it.name).or_else(|| cfg.colors.get(&it.category));
                hex(custom.map_or(palette[i % palette.len()], |s| s.as_str()))
            })
            .collect();
        let head_d = (l.label_size * 1.9).round().max(2.0);
        let heads = ds
            .items
            .iter()
            .map(|it| {
                let p = it.image.as_ref()?;
                load_circle_image(p, head_d as u32)
                    .map_err(|e| eprintln!("warning: image {}: {e:#}; using initials", p.display()))
                    .ok()
            })
            .collect();
        let (bg, fg) = ch.colors();
        Ok(LinePainter {
            ds: ds.clone(),
            cfg: cfg.clone(),
            ch: ch.clone(),
            l,
            text: Text::new(),
            colors,
            heads,
            head_d,
            scales,
            bg: hex(&bg),
            fg: hex(&fg),
            grid_a: if ch.theme == Theme::Light { 0.12 } else { 0.10 },
            last_strings: Vec::new(),
        })
    }

    fn txt(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align) {
        if s.is_empty() {
            return;
        }
        self.text.draw(pm, s, x, y, size, bold, color, align);
        self.last_strings.push(s.to_string());
    }

    fn fit(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> f32 {
        let w = self.text.measure(s, size, bold);
        if w <= max_w { size } else { (size * max_w / w).floor().max(4.0) }
    }

    fn px(&self, f: &LineFrame, t: f64, v: f64) -> (f32, f32) {
        let l = &self.l;
        let u = ((t - f.x_min) / (f.x_max - f.x_min).max(1e-9)) as f32;
        let w = (v.max(0.0) / f.y_max.max(1e-9)) as f32;
        (l.chart_left + u * (l.chart_right - l.chart_left), l.chart_bottom - w * (l.chart_bottom - l.chart_top))
    }

    fn hline(&self, pm: &mut Pixmap, x0: f32, x1: f32, y: f32, c: Color) {
        let mut p = Paint::default();
        p.set_color(c);
        let th = (self.l.height as f32 / 1080.0).max(1.0);
        if let Some(r) = Rect::from_xywh(x0, y, x1 - x0, th) {
            pm.fill_rect(r, &p, Transform::identity(), None);
        }
    }

    /// Paint one frame. `header` = title, subtitle and date counter (off for thumbnails).
    pub fn paint(&mut self, pm: &mut Pixmap, f: &LineFrame, header: bool) {
        self.last_strings.clear();
        pm.fill(self.bg);
        let l = self.l.clone();
        let (w, h) = (l.width as f32, l.height as f32);
        if header {
            let max_w = if l.portrait { w - 2.0 * l.title_pos.0 } else { 0.70 * w };
            let (title, sub) = (self.cfg.title.clone(), self.cfg.subtitle.clone());
            let ts = self.fit(&title, l.title_size, true, max_w);
            self.txt(pm, &title, l.title_pos.0, l.title_pos.1, ts, true, self.fg, Align::Left);
            let ss = self.fit(&sub, l.subtitle_size, false, max_w);
            let sy = l.title_pos.1 + l.title_size * 0.9 + l.subtitle_size * 0.6;
            self.txt(pm, &sub, l.title_pos.0, sy, ss, false, with_alpha(self.fg, 0.7), Align::Left);
            let date = format_time(f.t, self.ds.granularity);
            let (dx, dy, ds) = if l.portrait { (0.94 * w, 0.165 * h, 0.034 * h) } else { (0.95 * w, 0.09 * h, 0.075 * h) };
            let a = if self.ch.theme == Theme::Light { 0.35 } else { 0.25 };
            self.txt(pm, &date, dx, dy, ds, true, with_alpha(self.fg, a), Align::Right);
        }
        self.paint_grid(pm, f);
        self.paint_lines(pm, f);
        let name = self.ch.name.clone();
        if !name.is_empty() {
            let (wx, wy) = l.watermark_pos;
            self.txt(pm, &name, wx, wy, l.watermark_size, true, with_alpha(self.fg, 0.4), Align::Right);
        }
    }

    fn paint_grid(&mut self, pm: &mut Pixmap, f: &LineFrame) {
        let l = self.l.clone();
        let tick = l.label_size * 0.75;
        let grid = with_alpha(self.fg, self.grid_a);
        for v in nice_ticks(f.y_max) {
            let (_, y) = self.px(f, f.x_min, v);
            self.hline(pm, l.chart_left, l.chart_right, y, grid);
            let s = format_tick(v, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style);
            self.txt(pm, &s, l.chart_left - tick * 0.6, y, tick, false, with_alpha(self.fg, 0.5), Align::Right);
        }
        let max_ticks = if l.portrait { 4.0 } else { 7.0 };
        let step = year_step(f.x_max - f.x_min, max_ticks);
        let mut yr = (f.x_min / step).ceil() * step;
        let th = (l.height as f32 / 1080.0).max(1.0);
        while yr <= f.x_max + 1e-9 {
            let (x, _) = self.px(f, yr, 0.0);
            let mut p = Paint::default();
            p.set_color(with_alpha(self.fg, self.grid_a * 0.6));
            if let Some(r) = Rect::from_xywh(x, l.chart_top, th, l.chart_bottom - l.chart_top) {
                pm.fill_rect(r, &p, Transform::identity(), None);
            }
            self.txt(pm, &format!("{yr:.0}"), x, l.chart_bottom + tick * 1.3, tick, false, with_alpha(self.fg, 0.5), Align::Center);
            yr += step;
        }
        self.hline(pm, l.chart_left, l.chart_right, l.chart_bottom, with_alpha(self.fg, 0.35));
    }

    fn paint_lines(&mut self, pm: &mut Pixmap, f: &LineFrame) {
        let l = self.l.clone();
        let stroke = Stroke { width: 0.0055 * (l.width.min(l.height) as f32), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        // (item, head x, head y, value) of every visible line
        let mut heads: Vec<(usize, f32, f32, f64)> = Vec::new();
        for i in 0..self.ds.items.len() {
            let k = self.scales[i];
            if !k.is_finite() {
                continue;
            }
            let it = &self.ds.items[i];
            let (Some(first), Some(last)) = (it.points.first(), it.points.last()) else { continue };
            let t0 = f.x_min.max(first.t);
            let t1 = f.t.min(last.t);
            if t1 < t0 {
                continue;
            }
            let mut pb = PathBuilder::new();
            let (sx, sy) = self.px(f, t0, series_at(it, t0).unwrap_or(0.0) * k);
            pb.move_to(sx, sy);
            for p in it.points.iter().filter(|p| p.t > t0 && p.t < t1) {
                let (x, y) = self.px(f, p.t, p.value * k);
                pb.line_to(x, y);
            }
            let v = series_at(it, t1).unwrap_or(0.0) * k;
            let (hx, hy) = self.px(f, t1, v);
            pb.line_to(hx + 0.01, hy); // a zero-length path still draws a dot
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(self.colors[i]);
                paint.anti_alias = true;
                pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
            heads.push((i, hx, hy, v));
        }
        // heads and labels: highest value drawn last (on top)
        heads.sort_by(|a, b| a.3.total_cmp(&b.3));
        let size = l.label_size;
        let r = self.head_d / 2.0;
        let gap = size * 1.45;
        let wanted: Vec<f32> = heads.iter().map(|h| h.2).collect();
        let label_y = spread(&wanted, gap, l.chart_top + size * 0.5, l.chart_bottom - size * 0.2);
        for (k, &(i, hx, hy, v)) in heads.iter().enumerate() {
            let c = self.colors[i];
            match &self.heads[i] {
                Some(img) => {
                    let paint = PixmapPaint::default();
                    pm.draw_pixmap((hx - r).round() as i32, (hy - r).round() as i32, img.as_ref(), &paint, Transform::identity(), None);
                    ring(pm, hx, hy, r, c);
                }
                None => {
                    let mut paint = Paint::default();
                    paint.set_color(c);
                    paint.anti_alias = true;
                    if let Some(circle) = PathBuilder::from_circle(hx, hy, r) {
                        pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
                    }
                    let ini = initials(&self.ds.items[i].name);
                    self.txt(pm, &ini, hx, hy, r * 0.85, true, Color::WHITE, Align::Center);
                }
            }
            let ly = label_y[k];
            let lx = hx + r + size * 0.5;
            if (ly - hy).abs() > size * 0.3 {
                // label pushed away from its head: thin connector
                let mut pb = PathBuilder::new();
                pb.move_to(hx + r, hy);
                pb.line_to(lx - size * 0.15, ly);
                if let Some(p) = pb.finish() {
                    let mut paint = Paint::default();
                    paint.set_color(with_alpha(c, 0.6));
                    paint.anti_alias = true;
                    let s = Stroke { width: (l.height as f32 / 1080.0).max(1.0) * 1.5, ..Default::default() };
                    pm.stroke_path(&p, &paint, &s, Transform::identity(), None);
                }
            }
            let name = self.ds.items[i].name.clone();
            // rebased money reads best in whole units: $695, not $695.2
            let value = match self.cfg.rebase {
                Some(_) => format_tick(v.round(), &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style),
                None => format_value(v, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style),
            };
            let max_w = l.width as f32 - lx - l.width as f32 * 0.02;
            let ns = self.fit(&name, size, true, max_w * 0.55);
            self.txt(pm, &name, lx, ly, ns, true, c, Align::Left);
            let nw = self.text.measure(&name, ns, true);
            let vs = self.fit(&value, size, true, (max_w - nw - size * 0.4).max(size));
            self.txt(pm, &value, lx + nw + size * 0.4, ly, vs, true, self.fg, Align::Left);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Item, Point};

    fn item(name: &str, pts: &[(f64, f64)]) -> Item {
        Item {
            name: name.into(),
            category: String::new(),
            image: None,
            points: pts.iter().map(|&(t, value)| Point { t, value, estimated: false }).collect(),
            sources: vec![],
        }
    }

    fn ds() -> Dataset {
        Dataset::from_items(vec![
            item("Apple", &[(2010.0, 10.0), (2011.0, 20.0), (2012.0, 40.0)]),
            item("Microsoft", &[(2010.0, 50.0), (2011.0, 55.0), (2012.0, 60.0)]),
        ])
    }

    #[test]
    fn series_interpolates_and_stops() {
        let d = ds();
        assert_eq!(series_at(&d.items[0], 2010.5), Some(15.0));
        assert_eq!(series_at(&d.items[0], 2012.0), Some(40.0));
        assert_eq!(series_at(&d.items[0], 2012.5), None);
        assert_eq!(series_at(&d.items[0], 2009.9), None);
    }

    #[test]
    fn rebase_starts_every_line_at_the_same_value() {
        let d = ds();
        let k = scales(&d, 2010.0, Some(1000.0));
        assert_eq!(series_at(&d.items[0], 2010.0).unwrap() * k[0], 1000.0);
        assert_eq!(series_at(&d.items[1], 2010.0).unwrap() * k[1], 1000.0);
        assert_eq!(series_at(&d.items[0], 2012.0).unwrap() * k[0], 4000.0); // 4x
        // a trimmed start (the Short) rebases at the shown start
        let k = scales(&d, 2011.0, Some(1000.0));
        assert_eq!(series_at(&d.items[0], 2011.0).unwrap() * k[0], 1000.0);
        assert_eq!(scales(&d, 2010.0, None), vec![1.0, 1.0]);
    }

    #[test]
    fn y_axis_grows_and_never_cuts_a_line() {
        let d = ds();
        let mut tr = LineTracker::new(&d, 2010.0, 2012.0, Some(1000.0));
        let mut last = 0.0;
        for i in 0..=120 {
            let t = 2010.0 + 2.0 * i as f64 / 120.0;
            let f = tr.step(&d, t, 1.0 / 60.0);
            let vmax = tr.visible_max(&d, t);
            assert!(f.y_max >= vmax * 1.0199, "t={t}: {} < {vmax}", f.y_max);
            assert!(f.y_max >= last - 1e-9, "axis shrank at {t}");
            assert!(f.x_max >= t && f.x_min == 2010.0);
            last = f.y_max;
        }
        assert!((tr.visible_max(&d, 2012.0) - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn x_axis_has_a_minimum_span_then_follows_heads() {
        let d = ds();
        let mut tr = LineTracker::new(&d, 2010.0, 2012.0, None);
        let f = tr.step(&d, 2010.0, 0.016);
        assert!(f.x_max > f.x_min);
        let f = tr.step(&d, 2012.0, 0.016);
        assert_eq!(f.x_max, 2012.0);
    }

    #[test]
    fn spread_keeps_order_gap_and_bounds() {
        let ys = spread(&[100.0, 101.0, 102.0], 10.0, 0.0, 1000.0);
        assert_eq!(ys, vec![100.0, 110.0, 120.0]);
        let ys = spread(&[995.0, 990.0, 999.0], 10.0, 0.0, 1000.0);
        assert!(ys.iter().all(|y| *y <= 1000.0));
        let mut s = ys.clone();
        s.sort_by(f32::total_cmp);
        assert!(s.windows(2).all(|w| w[1] - w[0] >= 9.999), "{ys:?}");
        assert_eq!(spread(&[], 10.0, 0.0, 1.0), Vec::<f32>::new());
    }

    #[test]
    fn start_placeholder_filled() {
        let cfg = VideoConfig { subtitle: "Value of $1,000 invested in {start}".into(), ..Default::default() };
        let c = fill_start(&cfg, 2010.0, crate::data::Granularity::Month);
        assert_eq!(c.subtitle, "Value of $1,000 invested in Jan 2010");
    }

    #[test]
    fn rebased_labels_are_whole_dollars() {
        let d = Dataset::from_items(vec![item("A", &[(2010.0, 100.0), (2011.0, 69.52)])]);
        let cfg = VideoConfig { unit_prefix: "$".into(), rebase: Some(1000.0), ..Default::default() };
        let mut tr = LineTracker::new(&d, 2010.0, 2011.0, cfg.rebase);
        let mut p = LinePainter::new(&d, &cfg, &ChannelConfig::default(), layout(0.5, false), tr.scales().to_vec()).unwrap();
        let f = tr.step(&d, 2011.0, 1.0);
        let mut pm = Pixmap::new(p.l.width, p.l.height).unwrap();
        p.paint(&mut pm, &f, true);
        assert!(p.last_strings.iter().any(|s| s == "$695"), "{:?}", p.last_strings);
    }

    #[test]
    fn year_ticks_stay_few() {
        assert_eq!(year_step(1.5, 7.0), 1.0);
        assert_eq!(year_step(15.0, 7.0), 5.0);
        assert_eq!(year_step(40.0, 4.0), 10.0);
    }

    fn painter(portrait: bool) -> (LinePainter, LineTracker, Dataset) {
        let d = ds();
        let cfg = VideoConfig { title: "Apple vs Microsoft".into(), unit_prefix: "$".into(), rebase: Some(1000.0), ..Default::default() };
        let tr = LineTracker::new(&d, 2010.0, 2012.0, cfg.rebase);
        let p = LinePainter::new(&d, &cfg, &ChannelConfig::default(), layout(0.5, portrait), tr.scales().to_vec()).unwrap();
        (p, tr, d)
    }

    fn count(pm: &Pixmap, c: &str) -> usize {
        let c = hex(c);
        let (r, g, b) = ((c.red() * 255.0) as i32, (c.green() * 255.0) as i32, (c.blue() * 255.0) as i32);
        pm.pixels().iter().filter(|p| (p.red() as i32 - r).abs() < 8 && (p.green() as i32 - g).abs() < 8 && (p.blue() as i32 - b).abs() < 8).count()
    }

    #[test]
    fn paints_both_lines_labels_and_values() {
        for portrait in [false, true] {
            let (mut p, mut tr, d) = painter(portrait);
            let f = tr.step(&d, 2012.0, 1.0);
            let mut pm = Pixmap::new(p.l.width, p.l.height).unwrap();
            p.paint(&mut pm, &f, true);
            assert!(count(&pm, LIGHT[0]) > 200, "apple line");
            assert!(count(&pm, LIGHT[1]) > 200, "microsoft line");
            let s = p.last_strings.join("|");
            for want in ["Apple vs Microsoft", "Apple", "Microsoft", "$4,000", "$1,200", "2011"] {
                assert!(s.contains(want), "{want} missing in {s}");
            }
        }
    }

    #[test]
    fn thumbnail_mode_has_no_header() {
        let (mut p, mut tr, d) = painter(false);
        let f = tr.step(&d, 2012.0, 1.0);
        let mut pm = Pixmap::new(p.l.width, p.l.height).unwrap();
        p.paint(&mut pm, &f, false);
        assert!(!p.last_strings.iter().any(|s| s == "Apple vs Microsoft"));
    }

    #[test]
    fn first_frame_draws_dots_without_panicking() {
        let (mut p, mut tr, d) = painter(true);
        let f = tr.step(&d, 2010.0, 1.0 / 60.0);
        let mut pm = Pixmap::new(p.l.width, p.l.height).unwrap();
        p.paint(&mut pm, &f, true);
        // both heads at $1,000 (plus possibly the $1,000 gridline label)
        assert!(p.last_strings.iter().filter(|s| *s == "$1,000").count() >= 2, "{:?}", p.last_strings);
    }
}
