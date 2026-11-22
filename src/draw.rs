use crate::config::{ChannelConfig, VideoConfig};
use crate::data::Dataset;
use crate::events::Event;
use crate::fmt::format_value;
use crate::layout::Layout;
use crate::scene::{BarState, FrameState};
use crate::text::{Align, Text};
use crate::timeline::Phase;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;
use tiny_skia::{Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

pub(crate) const PALETTE: [&str; 12] = [
    "#38bdf8", "#f472b6", "#facc15", "#4ade80", "#fb923c", "#a78bfa", "#f87171", "#2dd4bf", "#e879f9", "#a3e635",
    "#60a5fa", "#fbbf24",
];

pub fn hex(s: &str) -> Color {
    let s = s.trim().trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(s.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    Color::from_rgba8(c(0), c(2), c(4), 255)
}

pub(crate) fn with_alpha(mut c: Color, a: f32) -> Color {
    c.set_alpha((c.alpha() * a).clamp(0.0, 1.0));
    c
}

/// Per-frame overlays: (index, fade alpha) of the caption event and the leader item.
#[derive(Debug, Clone, Copy, Default)]
pub struct Overlay {
    pub caption: Option<(usize, f32)>,
    pub leader: Option<(usize, f32)>,
}

pub struct Painter {
    ds: Dataset,
    cfg: VideoConfig,
    ch: ChannelConfig,
    layout: Layout,
    text: Text,
    item_colors: Vec<Color>,
    images: Vec<Option<Pixmap>>,
    /// Same images at leader-badge size.
    badges: Vec<Option<Pixmap>>,
    events: Vec<Event>,
    bg: Color,
    fg: Color,
    grid_a: f32,
    year_a: f32,
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
        let item_colors = ds
            .items
            .iter()
            .map(|it| {
                let from_image = || {
                    let im = image::open(it.image.as_ref()?).ok()?.to_rgba8();
                    let [r, g, b] = dominant_color(&im)?;
                    Some(Color::from_rgba8(r, g, b, 255))
                };
                let overridden = cfg.colors.contains_key(&it.category);
                match cfg.bar_colors {
                    crate::config::BarColors::Image if !overridden => from_image(),
                    _ => None,
                }
                .unwrap_or(cat_colors[it.category.as_str()])
            })
            .collect();
        let load_all = |d: u32, warn: bool| -> Vec<Option<Pixmap>> {
            ds.items
                .iter()
                .map(|it| {
                    let path = it.image.as_ref()?;
                    match load_circle_image(path, d) {
                        Ok(pm) => Some(pm),
                        Err(e) => {
                            if warn {
                                eprintln!("warning: image {}: {e:#}; using initials", path.display());
                            }
                            None
                        }
                    }
                })
                .collect()
        };
        let images = load_all(layout.bar_height().round().max(1.0) as u32, true);
        let badges = load_all(layout.badge_box().2.round().max(1.0) as u32, false);
        Ok(Painter {
            ds: ds.clone(),
            cfg: cfg.clone(),
            ch: ch.clone(),
            layout,
            text: Text::new(),
            item_colors,
            images,
            badges,
            events: Vec::new(),
            bg: hex(&ch.colors().0),
            fg: hex(&ch.colors().1),
            grid_a: if ch.theme == crate::config::Theme::Light { 0.12 } else { 0.10 },
            year_a: if ch.theme == crate::config::Theme::Light { 0.35 } else { 0.25 },
            last_strings: Vec::new(),
        })
    }

    #[cfg(test)]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn panel(&self, pm: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32), a: f32) {
        let mut paint = Paint::default();
        paint.set_color(with_alpha(self.bg, 0.85 * a));
        paint.anti_alias = true;
        let r = (h * 0.12).min(w * 0.12);
        if let Some(path) = rounded_rect(x, y, w, h, r) {
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub fn paint_caption(&mut self, pm: &mut Pixmap, i: usize, a: f32) {
        let Some(ev) = self.events.get(i).cloned() else { return };
        let b @ (x, y, w, h) = self.layout.caption_box();
        self.panel(pm, b, a);
        let pad = w * 0.04;
        let (cx, inner) = (x + w / 2.0, w - 2.0 * pad);
        let red = with_alpha(hex("#e11d48"), a);
        let lines = [(&ev.headline, 0.20, 0.16, true, red), (&ev.big, 0.50, 0.42, true, with_alpha(self.fg, a)), (&ev.caption, 0.82, 0.16, false, with_alpha(self.fg, 0.85 * a))];
        for (s, yf, hf, bold, color) in lines {
            let size = self.fit_size(s, h * hf, bold, inner);
            self.txt(pm, s, cx, y + h * yf, size, bold, color, Align::Center);
        }
    }

    fn paint_badge(&mut self, pm: &mut Pixmap, i: usize, a: f32) {
        let (x, y, s, _) = self.layout.badge_box();
        match &self.badges[i] {
            Some(img) => {
                let paint = PixmapPaint { opacity: a, ..Default::default() };
                pm.draw_pixmap(x.round() as i32, y.round() as i32, img.as_ref(), &paint, Transform::identity(), None);
                ring(pm, x + s / 2.0, y + s / 2.0, s / 2.0, with_alpha(self.fg, 0.2 * a));
            }
            None => {
                let c = self.item_colors[i];
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba(c.red() * 0.55, c.green() * 0.55, c.blue() * 0.55, a).unwrap());
                paint.anti_alias = true;
                let circle = PathBuilder::from_circle(x + s / 2.0, y + s / 2.0, s / 2.0).unwrap();
                pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
                let ini = initials(&self.ds.items[i].name);
                self.txt(pm, &ini, x + s / 2.0, y + s / 2.0, s * 0.4, true, with_alpha(Color::WHITE, a), Align::Center);
            }
        }
    }

    /// Thumbnail: race frame without the small title block, oversized title in the header area.
    pub fn paint_thumb(&mut self, pm: &mut Pixmap, fs: &FrameState) {
        self.last_strings.clear();
        pm.fill(self.bg);
        self.paint_race_inner(pm, fs, false);
        if let Some(i) = fs.leader {
            self.paint_badge(pm, i, 1.0);
        }
        let l = self.layout.clone();
        let title = self.cfg.title.clone();
        let size = self.fit_size(&title, l.title_size * 2.2, true, l.width as f32 * 0.92);
        self.txt(pm, &title, l.width as f32 / 2.0, l.chart_top * 0.45, size, true, self.fg, Align::Center);
    }

    /// Largest size ≤ `size` at which `s` fits in `max_w`.
    fn fit_size(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> f32 {
        let w = self.text.measure(s, size, bold);
        // floor at 4 px: cosmic-text panics on a zero line height
        if w <= max_w { size } else { (size * max_w / w).floor().max(4.0) }
    }

    #[allow(clippy::too_many_arguments)]
    fn txt(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align) {
        if s.is_empty() {
            return;
        }
        self.text.draw(pm, s, x, y, size, bold, color, align);
        self.last_strings.push(s.to_string());
    }

    /// Draw only the intro or end-card overlay onto an existing frame (no clear).
    pub fn paint_card(&mut self, pm: &mut Pixmap, phase: &Phase) {
        self.last_strings.clear();
        match *phase {
            Phase::Intro { t } => self.paint_intro(pm, t),
            Phase::End { t } => self.paint_end(pm, t),
            Phase::Race { .. } | Phase::Hold => {}
        }
    }

    /// Big centred title in the header area (thumbnails).
    pub fn paint_title_overlay(&mut self, pm: &mut Pixmap, color: Color) {
        let l = self.layout.clone();
        let title = self.cfg.title.clone();
        let size = self.fit_size(&title, l.title_size * 2.2, true, l.width as f32 * 0.92);
        self.txt(pm, &title, l.width as f32 / 2.0, l.chart_top * 0.45, size, true, color, Align::Center);
    }

    pub fn set_events(&mut self, events: Vec<Event>) {
        self.events = events;
    }

    #[cfg(test)]
    pub fn paint(&mut self, pm: &mut Pixmap, phase: &Phase, fs: &FrameState) {
        self.paint_with(pm, phase, fs, &Overlay::default());
    }

    /// Paint a frame plus the caption / leader badge overlays (each with its fade alpha).
    pub fn paint_with(&mut self, pm: &mut Pixmap, phase: &Phase, fs: &FrameState, ov: &Overlay) {
        self.last_strings.clear();
        pm.fill(self.bg);
        self.paint_race(pm, fs);
        if let Some((i, a)) = ov.caption {
            self.paint_caption(pm, i, a);
        }
        if let Some((i, a)) = ov.leader {
            self.paint_badge(pm, i, a);
        }
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
        let a = (t as f32 / 0.12).min(1.0);
        self.dim(pm, 0.92 * a);
        let l = self.layout.clone();
        let fg = with_alpha(self.fg, a);
        let cx = l.width as f32 / 2.0;
        if !l.portrait {
            let line = l.label_size * 1.5;
            let mut y = l.height as f32 * 0.16;
            self.txt(pm, "Sources", cx, y, l.title_size * 0.9, true, fg, Align::Center);
            y += line * 1.4;
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
                y += line * 0.3;
                let note = format!("* estimated values: {:.0}%", share * 100.0);
                self.txt(pm, &note, cx, y, l.label_size, false, with_alpha(fg, 0.7), Align::Center);
            }
            let name = self.ch.name.clone();
            if !name.is_empty() {
                self.txt(pm, &name, cx, l.height as f32 * 0.9, l.title_size * 0.7, true, fg, Align::Center);
            }
        }
        self.paint_buttons(pm, &end_state(t), a);
    }

    /// Like and Subscribe button rectangles (x, y, w, h) at full size.
    pub fn button_rects(&self) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
        let (w, h) = (self.layout.width as f32, self.layout.height as f32);
        let (bh, like_w, sub_w, gap, cy) = if self.layout.portrait {
            (0.05 * h, 0.26 * w, 0.42 * w, 0.03 * w, 0.48 * h)
        } else {
            (0.09 * h, 0.16 * w, 0.25 * w, 0.02 * w, 0.66 * h)
        };
        let bell = bh * 1.1;
        let total = like_w + gap + sub_w + gap + bell;
        let x0 = (w - total) / 2.0;
        ((x0, cy - bh / 2.0, like_w, bh), (x0 + like_w + gap, cy - bh / 2.0, sub_w, bh))
    }

    fn paint_buttons(&mut self, pm: &mut Pixmap, st: &EndState, a: f32) {
        let (like, sub) = self.button_rects();
        let scale = |r: (f32, f32, f32, f32)| {
            let (cx, cy) = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
            let (w, h) = (r.2 * st.pop, r.3 * st.pop);
            (cx - w / 2.0, cy - h / 2.0, w, h)
        };
        if st.pop <= 0.01 {
            return;
        }
        let fill = |pm: &mut Pixmap, r: (f32, f32, f32, f32), c: Color| {
            let mut paint = Paint::default();
            paint.set_color(with_alpha(c, a));
            paint.anti_alias = true;
            if let Some(p) = rounded_rect(r.0, r.1, r.2, r.3, r.3 / 2.0) {
                pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
            }
        };
        // Like
        let lr = scale(like);
        let (like_bg, like_fg) = if st.liked { (hex("#2563eb"), Color::WHITE) } else { (hex("#e5e7eb"), hex("#111827")) };
        fill(pm, lr, like_bg);
        let size = lr.3 * 0.42;
        let icon = lr.3 * 0.5;
        let ix = lr.0 + lr.2 * 0.22;
        let iy = lr.1 + lr.3 / 2.0;
        fill_path(pm, &thumb_path(ix, iy, icon), with_alpha(like_fg, a));
        self.txt(pm, "LIKE", ix + icon * 0.8, iy, size, true, with_alpha(like_fg, a), Align::Left);
        // Subscribe
        let sr = scale(sub);
        let (sub_bg, label) = if st.subscribed { (hex("#9ca3af"), "SUBSCRIBED") } else { (hex("#dc2626"), "SUBSCRIBE") };
        fill(pm, sr, sub_bg);
        let size = self.fit_size(label, sr.3 * 0.42, true, sr.2 * 0.84);
        self.txt(pm, label, sr.0 + sr.2 / 2.0, sr.1 + sr.3 / 2.0, size, true, with_alpha(Color::WHITE, a), Align::Center);
        // Bell, swinging about its top
        let bs = sr.3 * 0.9;
        let (bx, by) = (sr.0 + sr.2 + sr.3 * 0.25 + bs / 2.0, sr.1 + sr.3 * 0.05);
        let tr = Transform::from_rotate_at(st.bell_angle.to_degrees(), bx, by);
        let bell_color = if st.subscribed { hex("#f59e0b") } else { self.fg };
        let mut paint = Paint::default();
        paint.set_color(with_alpha(bell_color, a));
        paint.anti_alias = true;
        pm.fill_path(&bell_path(bx, by, bs), &paint, FillRule::Winding, tr, None);
        // Cursor
        if let Some((fx, fy)) = st.cursor {
            let (l0, s0) = ((like.0 + like.2 / 2.0), (sub.0 + sub.2 / 2.0));
            let x = l0 + (s0 - l0) * fx;
            let y = like.1 + like.3 / 2.0 + like.3 * fy;
            let path = cursor_path(x, y, like.3 * 0.55);
            fill_path(pm, &path, with_alpha(Color::WHITE, a));
            let mut paint = Paint::default();
            paint.set_color(with_alpha(Color::BLACK, a));
            paint.anti_alias = true;
            let stroke = tiny_skia::Stroke { width: (like.3 * 0.035).max(1.0), ..Default::default() };
            pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
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
        paint.set_color(with_alpha(self.fg, self.grid_a));
        let tick_size = l.label_size * 0.75;
        for v in nice_ticks(axis_max) {
            let x = l.chart_left + (v / axis_max) as f32 * chart_w;
            if let Some(r) = Rect::from_xywh(x, l.chart_top - tick_size * 0.4, (l.height as f32 / 1080.0).max(1.0), l.chart_bottom - l.chart_top) {
                pm.fill_rect(r, &paint, Transform::identity(), None);
            }
            let label = crate::fmt::format_tick(v, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style);
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
        let (yx, yy, yw, yh) = l.year_box();
        let ys = self.fit_size(&year, l.year_size.min(yh / 1.2), true, yw);
        let (x, align) = (yx + yw, Align::Right);
        self.txt(pm, &year, x, yy + yh / 2.0, ys, true, with_alpha(self.fg, self.year_a), align);
        self.paint_axis(pm, fs.axis_max);
        // bars, bottom-most first so overtaking bars draw on top
        for b in fs.bars.iter().rev() {
            self.paint_bar(pm, b, fs.axis_max);
        }
        // watermark
        let name = self.ch.name.clone();
        if !name.is_empty() {
            let (wx, wy) = l.watermark_pos;
            self.txt(pm, &name, wx, wy, l.watermark_size, true, with_alpha(self.fg, 0.4), Align::Right);
        }
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
                ring(pm, icx, cy, bh / 2.0, with_alpha(self.fg, 0.2 * b.alpha));
            }
            None => {
                let mut paint = Paint::default();
                let c = self.item_colors[b.idx];
                paint.set_color(Color::from_rgba(c.red() * 0.55, c.green() * 0.55, c.blue() * 0.55, b.alpha).unwrap());
                paint.anti_alias = true;
                pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
                let ini = initials(&self.ds.items[b.idx].name);
                self.txt(pm, &ini, icx, cy, bh * 0.42, true, with_alpha(Color::WHITE, b.alpha), Align::Center);
            }
        }
        let pad = label_size * 0.5;
        let name = self.ds.items[b.idx].name.clone();
        let name_w = self.text.measure(&name, label_size, true);
        let fg = with_alpha(self.fg, b.alpha);
        let mut after = icx + bh / 2.0 + pad;
        if name_w + 2.0 * pad <= bw {
            // inside the bar: always white for contrast against the bar colour
            let on_bar = label_color_on(self.item_colors[b.idx]);
            self.txt(pm, &name, x0 + pad, cy, label_size, true, with_alpha(on_bar, b.alpha), Align::Left);
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
pub(crate) fn initials(name: &str) -> String {
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

/// Most common colour of an image (4-bit buckets, averaged), ignoring transparent,
/// near-white and near-black pixels. None if nothing qualifies.
pub fn dominant_color(img: &image::RgbaImage) -> Option<[u8; 3]> {
    let mut buckets: std::collections::HashMap<u16, (u32, [u64; 3])> = std::collections::HashMap::new();
    for p in img.pixels() {
        let [r, g, b, a] = p.0;
        if a < 128 || (r > 225 && g > 225 && b > 225) || (r < 30 && g < 30 && b < 30) {
            continue;
        }
        let key = ((r as u16 >> 4) << 8) | ((g as u16 >> 4) << 4) | (b as u16 >> 4);
        let e = buckets.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        e.1[0] += r as u64;
        e.1[1] += g as u64;
        e.1[2] += b as u64;
    }
    // ties broken by bucket key so the choice is the same every run
    let (_, (n, sum)) = buckets.into_iter().max_by_key(|(k, (n, _))| (*n, *k))?;
    let n = n as u64;
    Some([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8])
}

/// Load an image, crop to a centred square, scale to d×d and cut to a circle (premultiplied).
pub(crate) fn load_circle_image(path: &Path, d: u32) -> Result<Pixmap> {
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

/// Animation state of the end card at fraction `t` (0..1) of its duration.
#[derive(Debug, Clone, Copy)]
pub struct EndState {
    pub pop: f32,
    pub liked: bool,
    pub subscribed: bool,
    pub bell_angle: f32,
    /// Cursor tip: x as a fraction between Like (0) and Subscribe (1) centres,
    /// y in button heights below the button centre.
    pub cursor: Option<(f32, f32)>,
}

pub fn end_state(t: f64) -> EndState {
    let t = t as f32;
    let ease_out_back = |x: f32| {
        let (c1, c3) = (1.70158, 2.70158);
        1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
    };
    let smooth = |x: f32| x * x * (3.0 - 2.0 * x);
    let pop = if t < 0.15 { ease_out_back(t / 0.15) } else { 1.0 };
    let cursor = if t < 0.15 || t >= 0.65 {
        None
    } else if t < 0.35 {
        let u = smooth((t - 0.15) / 0.2);
        Some((-0.6 + 0.6 * u, 1.6 - 1.4 * u))
    } else if t < 0.55 {
        Some((smooth((t - 0.35) / 0.2), 0.2))
    } else {
        Some((1.0, 0.2))
    };
    let bell_angle = if (0.6..0.9).contains(&t) {
        let u = (t - 0.6) / 0.3;
        0.35 * (std::f32::consts::TAU * 2.0 * u).sin() * (1.0 - u)
    } else {
        0.0
    };
    EndState { pop, liked: t >= 0.35, subscribed: t >= 0.55, bell_angle, cursor }
}

fn fill_path(pm: &mut Pixmap, path: &tiny_skia::Path, c: Color) {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Thumbs-up icon centred at (cx, cy), size s.
fn thumb_path(cx: f32, cy: f32, s: f32) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    let (x, y) = (cx - s / 2.0, cy - s / 2.0);
    // cuff
    pb.push_rect(Rect::from_xywh(x, y + s * 0.42, s * 0.22, s * 0.58).unwrap());
    // hand with raised thumb
    pb.move_to(x + s * 0.3, y + s * 0.42);
    pb.line_to(x + s * 0.52, y + s * 0.05);
    pb.quad_to(x + s * 0.68, y, x + s * 0.66, y + s * 0.2);
    pb.line_to(x + s * 0.6, y + s * 0.38);
    pb.line_to(x + s * 0.92, y + s * 0.38);
    pb.quad_to(x + s * 1.02, y + s * 0.42, x + s * 0.96, y + s * 0.55);
    pb.line_to(x + s * 0.86, y + s * 0.95);
    pb.quad_to(x + s * 0.82, y + s, x + s * 0.74, y + s);
    pb.line_to(x + s * 0.3, y + s);
    pb.close();
    pb.finish().unwrap()
}

/// Bell hanging from (cx, top), height s.
fn bell_path(cx: f32, top: f32, s: f32) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    let w = s * 0.8;
    pb.move_to(cx, top);
    pb.cubic_to(cx + w * 0.42, top, cx + w * 0.38, top + s * 0.55, cx + w / 2.0, top + s * 0.78);
    pb.line_to(cx - w / 2.0, top + s * 0.78);
    pb.cubic_to(cx - w * 0.38, top + s * 0.55, cx - w * 0.42, top, cx, top);
    pb.close();
    pb.push_circle(cx, top + s * 0.88, s * 0.1);
    pb.finish().unwrap()
}

/// Arrow cursor with its tip at (x, y), height s.
fn cursor_path(x: f32, y: f32, s: f32) -> tiny_skia::Path {
    let k = s / 20.0;
    let pts = [(0.0, 0.0), (0.0, 17.0), (4.0, 13.0), (7.0, 20.0), (10.0, 19.0), (7.0, 12.0), (12.0, 12.0)];
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    for (px, py) in &pts[1..] {
        pb.line_to(x + px * k, y + py * k);
    }
    pb.close();
    pb.finish().unwrap()
}

/// White on dark bars, near-black on light ones (relative luminance > 0.6).
pub fn label_color_on(bar: Color) -> Color {
    let lum = 0.2126 * bar.red() + 0.7152 * bar.green() + 0.0722 * bar.blue();
    if lum > 0.6 { hex("#111827") } else { Color::WHITE }
}

/// Thin outline so white parts of a flag stay visible on a light background.
pub(crate) fn ring(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = tiny_skia::Stroke { width: (r * 0.08).max(1.0), ..Default::default() };
    if let Some(c) = PathBuilder::from_circle(cx, cy, r) {
        pm.stroke_path(&c, &paint, &stroke, Transform::identity(), None);
    }
}

/// Rectangle with all four corners rounded by r.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    const K: f32 = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + r * K, y, x + w, y + r - r * K, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + r * K, x + w - r + r * K, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - r * K, y + h, x, y + h - r + r * K, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - r * K, x + r - r * K, y, x + r, y);
    pb.close();
    pb.finish()
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
        let mut p = Painter::new(&ds, &cfg, &ChannelConfig { theme: crate::config::Theme::Dark, ..Default::default() }, Layout::landscape(0.5, 10)).unwrap();
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
        let dark = ChannelConfig { theme: crate::config::Theme::Dark, ..Default::default() };
        let mut p = Painter::new(&ds, &cfg, &dark, Layout::portrait(0.5, 8)).unwrap();
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

    fn paint_fixture(ch: ChannelConfig) -> (Painter, Pixmap) {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2001.0);
        let mut p = Painter::new(&ds, &VideoConfig::default(), &ch, Layout::landscape(0.5, 10)).unwrap();
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2001.0 }, &fs);
        (p, pm)
    }

    #[test]
    fn light_theme_default_white() {
        let (_, pm) = paint_fixture(ChannelConfig::default());
        let px = pm.pixel(2, 2).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (255, 255, 255));
    }

    #[test]
    fn dark_theme_navy() {
        let ch = ChannelConfig { theme: crate::config::Theme::Dark, ..Default::default() };
        let (_, pm) = paint_fixture(ch);
        let px = pm.pixel(2, 2).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (0x0f, 0x17, 0x2a));
    }

    #[test]
    fn empty_name_draws_no_watermark() {
        let (p, _) = paint_fixture(ChannelConfig::default());
        assert!(!p.last_strings.iter().any(|s| s.is_empty() || s == "Stat Race"), "{:?}", p.last_strings);
    }

    #[test]
    fn background_override_wins() {
        let ch = ChannelConfig { background: Some("#112233".into()), ..Default::default() };
        let (_, pm) = paint_fixture(ch);
        let px = pm.pixel(2, 2).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (0x11, 0x22, 0x33));
    }

    fn img(pixels: &[[u8; 4]]) -> image::RgbaImage {
        let mut im = image::RgbaImage::new(pixels.len() as u32, 1);
        for (i, p) in pixels.iter().enumerate() {
            im.put_pixel(i as u32, 0, image::Rgba(*p));
        }
        im
    }

    #[test]
    fn dominant_ignores_white() {
        let mut px = vec![[255, 255, 255, 255]; 6];
        px.extend(vec![[255, 0, 0, 255]; 4]);
        assert_eq!(dominant_color(&img(&px)), Some([255, 0, 0]));
    }

    #[test]
    fn all_white_none() {
        assert_eq!(dominant_color(&img(&[[255, 255, 255, 255]; 10])), None);
        assert_eq!(dominant_color(&img(&[[0, 0, 0, 255]; 10])), None);
    }

    #[test]
    fn transparent_ignored() {
        let mut px = vec![[0, 0, 255, 0]; 8];
        px.extend(vec![[0, 200, 0, 255]; 2]);
        assert_eq!(dominant_color(&img(&px)), Some([0, 200, 0]));
    }

    fn one_item_png(color: [u8; 4], bar_colors: crate::config::BarColors) -> (Painter, Pixmap, f32) {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("c.png");
        image::RgbaImage::from_pixel(32, 32, image::Rgba(color)).save(&png).unwrap();
        let ds = one_item(Some(png));
        let fs = settle(&ds, 10, 2000.5);
        let cfg = VideoConfig { bar_colors, ..Default::default() };
        let mut p = Painter::new(&ds, &cfg, &ChannelConfig::default(), Layout::landscape(0.5, 10)).unwrap();
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2000.5 }, &fs);
        let y = p.layout().slot_center_y(0.0);
        (p, pm, y)
    }

    #[test]
    fn bar_uses_image_colour() {
        let (_, pm, y) = one_item_png([0, 160, 60, 255], crate::config::BarColors::Image);
        assert!(count_color_in_row(&pm, y as u32, [0, 160, 60]) > 100);
    }

    #[test]
    fn bar_falls_back_to_category() {
        let (_, pm, y) = one_item_png([255, 255, 255, 255], crate::config::BarColors::Image);
        assert!(count_color_in_row(&pm, y as u32, [0x38, 0xbd, 0xf8]) > 100, "white image → palette[0]");
        let (_, pm, y) = one_item_png([0, 160, 60, 255], crate::config::BarColors::Category);
        assert!(count_color_in_row(&pm, y as u32, [0x38, 0xbd, 0xf8]) > 100, "category mode ignores image");
    }

    fn fixture_events() -> Vec<crate::events::Event> {
        vec![crate::events::Event {
            from: 2000.0,
            to: 2003.0,
            headline: "US produces".into(),
            big: "90%".into(),
            caption: "World's Oil".into(),
        }]
    }

    #[test]
    fn caption_drawn_when_active() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2001.0);
        let mut p = painter(&ds);
        p.set_events(fixture_events());
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        let ov = Overlay { caption: Some((0, 1.0)), leader: None };
        p.paint_with(&mut pm, &Phase::Race { t: 2001.0 }, &fs, &ov);
        for s in ["US produces", "90%", "World's Oil"] {
            assert!(p.last_strings.iter().any(|x| x == s), "{s} missing: {:?}", p.last_strings);
        }
    }

    #[test]
    fn long_caption_fits_box() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let empty = crate::scene::FrameState { t: 2001.0, bars: vec![], axis_max: 1.0, leader: None };
        let mut p = painter(&ds);
        let mut ev = fixture_events();
        ev[0].caption = "an extremely long caption that goes on and on well past any sensible width for a box".into();
        ev[0].headline = "a very long headline that also does not fit at its natural size anywhere".into();
        p.set_events(ev);
        let (w, h) = (p.layout().width, p.layout().height);
        let mut without = Pixmap::new(w, h).unwrap();
        p.paint_with(&mut without, &Phase::Race { t: 2001.0 }, &empty, &Overlay::default());
        let mut with = Pixmap::new(w, h).unwrap();
        p.paint_with(&mut with, &Phase::Race { t: 2001.0 }, &empty, &Overlay { caption: Some((0, 1.0)), leader: None });
        let (bx, by, bw, bh) = p.layout().caption_box();
        for y in 0..h {
            for x in 0..w {
                if with.pixel(x, y) != without.pixel(x, y) {
                    let (fx, fy) = (x as f32, y as f32);
                    assert!(fx >= bx - 2.0 && fx <= bx + bw + 2.0 && fy >= by - 2.0 && fy <= by + bh + 2.0, "pixel {x},{y} outside caption box");
                }
            }
        }
    }

    #[test]
    fn badge_drawn_for_leader() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("red.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
        let ds = one_item(Some(png));
        let fs = settle(&ds, 10, 2000.5);
        let mut p = Painter::new(&ds, &VideoConfig { bar_colors: crate::config::BarColors::Category, ..Default::default() }, &ChannelConfig::default(), Layout::landscape(0.5, 10)).unwrap();
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint_with(&mut pm, &Phase::Race { t: 2000.5 }, &fs, &Overlay { caption: None, leader: Some((0, 1.0)) });
        let (bx, by, bw, bh) = p.layout().badge_box();
        let px = pm.pixel((bx + bw / 2.0) as u32, (by + bh / 2.0) as u32).unwrap();
        assert!(px.red() > 230 && px.green() < 30, "{px:?}");
    }

    #[test]
    fn end_state_sequence() {
        let s = end_state(0.0);
        assert!(s.pop < 0.05 && !s.liked && !s.subscribed);
        let s = end_state(0.2);
        assert!(s.pop > 0.99 && !s.liked);
        let s = end_state(0.4);
        assert!(s.liked && !s.subscribed);
        let s = end_state(0.6);
        assert!(s.liked && s.subscribed);
        assert!(end_state(0.7).bell_angle.abs() > 0.05);
        assert_eq!(end_state(0.95).bell_angle, 0.0);
        assert!(end_state(0.9).cursor.is_none());
        assert!(end_state(0.3).cursor.is_some());
    }

    fn end_painter(portrait: bool) -> (Painter, FrameState_) {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2003.0);
        let layout = if portrait { Layout::portrait(0.5, 12) } else { Layout::landscape(0.5, 10) };
        (Painter::new(&ds, &VideoConfig::default(), &ChannelConfig::default(), layout).unwrap(), fs)
    }
    type FrameState_ = crate::scene::FrameState;

    fn paint_end_at(p: &mut Painter, fs: &FrameState_, t: f64) -> Pixmap {
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::End { t }, fs);
        pm
    }

    #[test]
    fn end_card_strings() {
        let (mut p, fs) = end_painter(false);
        paint_end_at(&mut p, &fs, 0.3);
        for s in ["LIKE", "SUBSCRIBE", "Sources"] {
            assert!(p.last_strings.iter().any(|x| x == s), "{s}: {:?}", p.last_strings);
        }
        paint_end_at(&mut p, &fs, 0.7);
        assert!(p.last_strings.iter().any(|x| x == "SUBSCRIBED"));
        let (mut p, fs) = end_painter(true);
        paint_end_at(&mut p, &fs, 0.3);
        assert!(p.last_strings.iter().any(|x| x == "SUBSCRIBE"));
        assert!(!p.last_strings.iter().any(|x| x == "Sources"));
    }

    fn near(px: tiny_skia::PremultipliedColorU8, hexs: &str) -> bool {
        let c = hex(hexs).to_color_u8();
        (px.red() as i32 - c.red() as i32).abs() <= 10
            && (px.green() as i32 - c.green() as i32).abs() <= 10
            && (px.blue() as i32 - c.blue() as i32).abs() <= 10
    }

    #[test]
    fn liked_button_turns_blue() {
        let (mut p, fs) = end_painter(false);
        let pm = paint_end_at(&mut p, &fs, 0.5);
        let ((x, y, w, h), _) = p.button_rects();
        let px = pm.pixel((x + w * 0.06) as u32, (y + h * 0.5) as u32).unwrap();
        assert!(near(px, "#2563eb"), "{px:?}");
    }

    #[test]
    fn subscribed_button_turns_grey() {
        let (mut p, fs) = end_painter(false);
        let (_, (x, y, w, h)) = p.button_rects();
        let probe = ((x + w * 0.04) as u32, (y + h * 0.5) as u32);
        let pm = paint_end_at(&mut p, &fs, 0.3);
        assert!(near(pm.pixel(probe.0, probe.1).unwrap(), "#dc2626"));
        let pm = paint_end_at(&mut p, &fs, 0.7);
        assert!(near(pm.pixel(probe.0, probe.1).unwrap(), "#9ca3af"));
    }

    #[test]
    fn huge_caption_does_not_panic() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let fs = settle(&ds, 10, 2001.0);
        let mut p = painter(&ds);
        let mut ev = fixture_events();
        ev[0].caption = "x".repeat(3000);
        ev[0].headline = "y".repeat(3000);
        p.set_events(ev);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint_with(&mut pm, &Phase::Race { t: 2001.0 }, &fs, &Overlay { caption: Some((0, 1.0)), leader: None });
        assert!(p.fit_size("z", 40.0, true, 0.5) >= 4.0);
    }

    #[test]
    fn label_dark_on_light_bar() {
        assert_eq!(label_color_on(hex("#facc15")), hex("#111827"));
        assert_eq!(label_color_on(hex("#1d4ed8")), Color::WHITE);
    }

    #[test]
    fn image_circle_has_ring_on_light_theme() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("white.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255])).save(&png).unwrap();
        let ds = one_item(Some(png));
        let fs = settle(&ds, 10, 2000.5);
        let mut p = painter(&ds);
        let mut pm = Pixmap::new(p.layout().width, p.layout().height).unwrap();
        p.paint(&mut pm, &Phase::Race { t: 2000.5 }, &fs);
        let (cx, cy) = p.bar_tip(&fs.bars[0], fs.axis_max);
        let r = p.layout().bar_height() / 2.0;
        // scan across the circle's top edge: some pixel must be darker than white
        let darker = (-2..=2).any(|d| {
            let px = pm.pixel(cx as u32, (cy - r + d as f32) as u32).unwrap();
            px.red() < 235
        });
        assert!(darker, "white flag edge invisible on white background");
    }

    #[test]
    fn paint_card_leaves_race_untouched() {
        let ds = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        let mut p = painter(&ds);
        let (w, h) = (p.layout().width, p.layout().height);
        let mut pm = Pixmap::new(w, h).unwrap();
        pm.fill(Color::from_rgba8(10, 200, 30, 255));
        let before = pm.clone();
        p.paint_card(&mut pm, &Phase::Race { t: 2001.0 });
        assert!(pm == before, "race phase must not draw");
        p.paint_card(&mut pm, &Phase::End { t: 0.5 });
        assert!(p.last_strings.iter().any(|s| s == "SUBSCRIBE"), "{:?}", p.last_strings);
    }
}
