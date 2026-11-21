//! Pillar walk scene: sky, clouds, skyline, glossy floor, 3D pillars with reflections.

use crate::camera::{focus_x, Camera, PHOTO_H, PILLAR_D, PILLAR_W, SPACING};
use crate::fmt::format_tick;
use crate::text::{Align, Text};
use std::path::Path;
use crate::config::{PillarColors, VideoConfig};
use crate::draw::{dominant_color, hex};
use crate::pillars::Pillar;
use anyhow::Result;
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Rect, Shader, SpreadMode, Transform};

pub const SKY_TOP: &str = "#4f8fdc";
pub const SKY_HORIZON: &str = "#cfe3f7";
pub const SKYLINE: &str = "#3b4a63";
pub const FLOOR_HORIZON: &str = "#2b3a50";
pub const FLOOR: &str = "#1e2a3a";
pub const PLATE: &str = "#111827";
pub const GOLD: &str = "#facc15";
/// Plate (text block on the front face) height as a fraction of pillar width.
pub const PLATE_H: f32 = 0.55;
/// Smallest drawn pillar height (world units): the plate plus a margin.
pub const MIN_H: f32 = PLATE_H * PILLAR_W * 1.15;
const PHOTO_PX: (u32, u32) = (400, 500);

pub struct WalkPainter {
    rows: Vec<Pillar>,
    colors: Vec<Color>,
    photos: Vec<Option<Pixmap>>,
    /// Sky + floor gradients: fixed (the horizon never moves), drawn once.
    backdrop: Pixmap,
    cfg: VideoConfig,
    text: Text,
    width: u32,
    height: u32,
    portrait: bool,
    /// Strings drawn by the last `paint` (tests).
    pub last_strings: Vec<String>,
}

fn shade(c: Color, k: f32) -> Color {
    Color::from_rgba((c.red() * k).min(1.0), (c.green() * k).min(1.0), (c.blue() * k).min(1.0), c.alpha()).unwrap()
}

fn fill(pm: &mut Pixmap, path: &tiny_skia::Path, c: Color) {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn quad(pts: [(f32, f32); 4]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish()
}

fn vgradient(pm: &mut Pixmap, y0: f32, y1: f32, top: Color, bottom: Color) {
    let w = pm.width() as f32;
    let Some(rect) = Rect::from_ltrb(0.0, y0.max(0.0), w, y1.min(pm.height() as f32)) else { return };
    let shader = LinearGradient::new(
        Point::from_xy(0.0, y0),
        Point::from_xy(0.0, y1),
        vec![GradientStop::new(0.0, top), GradientStop::new(1.0, bottom)],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(top));
    let paint = Paint { shader, anti_alias: false, ..Default::default() };
    pm.fill_rect(rect, &paint, Transform::identity(), None);
}

/// Photo cropped to 4:5 and scaled to PHOTO_PX, premultiplied.
fn load_photo(path: &Path) -> Result<Pixmap> {
    let img = image::open(path)?.resize_to_fill(PHOTO_PX.0, PHOTO_PX.1, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut pm = Pixmap::new(PHOTO_PX.0, PHOTO_PX.1).unwrap();
    for (dst, src) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        let pre = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *dst = tiny_skia::PremultipliedColorU8::from_rgba(pre(r), pre(g), pre(b), a).unwrap();
    }
    Ok(pm)
}

/// Tiny deterministic generator for clouds and skyline (same scene every render).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }
}

impl WalkPainter {
    pub fn new(rows: &[Pillar], cfg: &VideoConfig, width: u32, height: u32, portrait: bool) -> Result<WalkPainter> {
        let base = hex(&cfg.pillar_color);
        let colors = rows
            .iter()
            .map(|r| {
                let from_image = || {
                    let im = image::open(r.image.as_ref()?).ok()?.to_rgba8();
                    let [cr, cg, cb] = dominant_color(&im)?;
                    Some(Color::from_rgba8(cr, cg, cb, 255))
                };
                match cfg.pillar_colors {
                    PillarColors::Image => from_image().unwrap_or(base),
                    PillarColors::Single => base,
                }
            })
            .collect();
        let photos = rows
            .iter()
            .map(|r| {
                let path = r.image.as_ref()?;
                match load_photo(path) {
                    Ok(pm) => Some(pm),
                    Err(e) => {
                        eprintln!("warning: image {}: {e:#}; using initials", path.display());
                        None
                    }
                }
            })
            .collect();
        let horizon = Camera::for_frame(width, height, portrait).horizon;
        let mut backdrop = Pixmap::new(width, height).ok_or_else(|| anyhow::anyhow!("bad frame size"))?;
        vgradient(&mut backdrop, 0.0, horizon, hex(SKY_TOP), hex(SKY_HORIZON));
        vgradient(&mut backdrop, horizon, height as f32, hex(FLOOR_HORIZON), hex(FLOOR));
        Ok(WalkPainter {
            rows: rows.to_vec(),
            colors,
            photos,
            backdrop,
            cfg: cfg.clone(),
            text: Text::new(),
            width,
            height,
            portrait,
            last_strings: Vec::new(),
        })
    }

    #[cfg(test)]
    pub fn pillar_color(&self, i: usize) -> Color {
        self.colors[i]
    }

    fn camera(&self, p: f64) -> Camera {
        let mut c = Camera::for_frame(self.width, self.height, self.portrait);
        c.x = focus_x(p, self.portrait);
        c
    }

    fn height_of(&self, i: usize, scale: f32) -> f32 {
        (self.rows[i].value as f32 * scale).max(MIN_H)
    }

    /// Front face of pillar `i` on screen (x, y, w, h).
    pub fn pillar_rect(&self, i: usize, p: f64, scale: f32) -> Option<(f32, f32, f32, f32)> {
        let cam = self.camera(p);
        let x0 = i as f32 * SPACING - PILLAR_W / 2.0;
        let h = self.height_of(i, scale);
        let (l, t) = cam.project(x0, h, 0.0)?;
        let (r, b) = cam.project(x0 + PILLAR_W, 0.0, 0.0)?;
        Some((l, t, r - l, b - t))
    }

    pub fn paint(&mut self, pm: &mut Pixmap, p: f64, scale: f32) {
        self.last_strings.clear();
        let cam = self.camera(p);
        let (w, h) = (self.width as f32, self.height as f32);
        // sky and floor
        pm.data_mut().copy_from_slice(self.backdrop.data());
        let px_per_unit = cam.focal / cam.dist;
        self.paint_clouds(pm, cam.x * px_per_unit * 0.05, cam.horizon);
        self.paint_skyline(pm, cam.x * px_per_unit * 0.25, cam.horizon);
        // visible pillars, farthest from the camera first
        let mut vis: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                let (sx, _) = cam.project(i as f32 * SPACING, 0.0, 0.0).unwrap_or((-1e9, 0.0));
                sx > -0.5 * w && sx < 1.5 * w
            })
            .collect();
        vis.sort_by(|&a, &b| {
            let d = |i: usize| (i as f32 * SPACING - cam.x).abs();
            d(b).total_cmp(&d(a))
        });
        for &i in &vis {
            self.paint_reflection(pm, &cam, i, scale);
        }
        for &i in &vis {
            self.paint_pillar(pm, &cam, i, scale);
            self.paint_plate_and_photo(pm, i, p, scale);
        }
        let title = self.cfg.title.clone();
        if !title.is_empty() {
            let size = 0.035 * h;
            let size = self.fit(&title, size, true, w * 0.9);
            self.txt(pm, &title, 0.04 * w, 0.06 * h, size, true, Color::WHITE, Align::Left);
        }
    }

    fn txt(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align) {
        if s.is_empty() {
            return;
        }
        self.text.draw(pm, s, x, y, size, bold, color, align);
        self.last_strings.push(s.to_string());
    }

    /// Largest size ≤ `size` fitting `max_w` (never below 4 px).
    fn fit(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> f32 {
        let w = self.text.measure(s, size, bold);
        if w <= max_w { size } else { (size * max_w / w).floor().max(4.0) }
    }

    /// Shrink to fit, but not below 55% of `size`; past that, cut the text with "…".
    fn fit_line(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> (String, f32) {
        let min = (size * 0.55).max(4.0);
        let fitted = self.fit(s, size, bold, max_w);
        if fitted >= min {
            return (s.to_string(), fitted);
        }
        let chars: Vec<char> = s.chars().collect();
        let mut n = chars.len();
        while n > 1 {
            n = n * 9 / 10;
            let cut: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "…";
            if self.text.measure(&cut, min, bold) <= max_w {
                return (cut, min);
            }
        }
        ("…".into(), min)
    }

    fn paint_plate_and_photo(&mut self, pm: &mut Pixmap, i: usize, p: f64, scale: f32) {
        let Some((x, y, fw, _)) = self.pillar_rect(i, p, scale) else { return };
        let row = self.rows[i].clone();
        let inner = fw * 0.84;
        let cx = x + fw / 2.0;
        // dark panel: name + value
        let (py, ph) = (y + fw * 0.05, fw * 0.27);
        if let Some(r) = Rect::from_xywh(x + fw * 0.08, py, inner, ph) {
            let mut paint = Paint::default();
            paint.set_color(hex(PLATE));
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        let (line, s) = self.fit_line(&row.name, fw * 0.1, true, inner * 0.92);
        self.txt(pm, &line, cx, py + ph * 0.3, s, true, Color::WHITE, Align::Center);
        // static values: whole numbers need no decimal
        let mut value = format_tick(row.value, &self.cfg.unit_prefix, &self.cfg.unit_suffix, self.cfg.number_style);
        if row.estimated {
            value.push('*');
        }
        let (line, s) = self.fit_line(&value, fw * 0.1, true, inner * 0.92);
        self.txt(pm, &line, cx, py + ph * 0.72, s, true, hex(GOLD), Align::Center);
        let (line, s) = self.fit_line(&row.detail, fw * 0.065, false, inner);
        self.txt(pm, &line, cx, y + fw * 0.39, s, false, Color::from_rgba(1.0, 1.0, 1.0, 0.85).unwrap(), Align::Center);
        let (line, s) = self.fit_line(&row.label, fw * 0.11, true, inner);
        self.txt(pm, &line, cx, y + fw * 0.49, s, true, hex(PLATE), Align::Center);
        // photo billboard standing on the pillar top
        let ph = fw * PHOTO_H / PILLAR_W;
        match &self.photos[i] {
            Some(img) => {
                let sx = fw / img.width() as f32;
                let sy = ph / img.height() as f32;
                let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
                pm.draw_pixmap(0, 0, img.as_ref(), &paint, Transform::from_row(sx, 0.0, 0.0, sy, x, y - ph), None);
            }
            None => {
                let tile = Rect::from_xywh(x + fw * 0.1, y - ph * 0.85, fw * 0.8, ph * 0.8);
                if let Some(r) = tile {
                    let mut paint = Paint::default();
                    paint.set_color(shade(self.colors[i], 0.6));
                    pm.fill_rect(r, &paint, Transform::identity(), None);
                }
                let ini: String = row.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect();
                self.txt(pm, &ini, cx, y - ph * 0.45, fw * 0.3, true, Color::WHITE, Align::Center);
            }
        }
    }

    fn paint_clouds(&self, pm: &mut Pixmap, shift: f32, horizon: f32) {
        let (w, _) = (self.width as f32, self.height as f32);
        let band = w * 1.6;
        let mut rng = Lcg(7);
        for _ in 0..9 {
            let bx = rng.next() * band;
            let y = horizon * (0.12 + 0.5 * rng.next());
            let size = w * (0.03 + 0.04 * rng.next());
            let x = (bx - shift).rem_euclid(band) - 0.3 * w;
            let mut pb = PathBuilder::new();
            for k in 0..4 {
                let r = size * (0.6 + 0.4 * ((k * 37 % 5) as f32 / 5.0));
                pb.push_circle(x + k as f32 * size * 0.7, y - (k % 2) as f32 * size * 0.3, r);
            }
            if let Some(path) = pb.finish() {
                fill(pm, &path, Color::from_rgba(1.0, 1.0, 1.0, 0.55).unwrap());
            }
        }
    }

    fn paint_skyline(&self, pm: &mut Pixmap, shift: f32, horizon: f32) {
        let (w, h) = (self.width as f32, self.height as f32);
        let band = w * 2.0;
        let mut rng = Lcg(42);
        let mut bx = 0.0;
        let color = hex(SKYLINE);
        let window = Color::from_rgba8(0xfd, 0xe6, 0x8a, 120);
        while bx < band {
            let bw = w * (0.025 + 0.04 * rng.next());
            let bh = h * (0.03 + 0.12 * rng.next());
            for copy in 0..2 {
                let x = (bx - shift).rem_euclid(band) - band * copy as f32;
                if x + bw < 0.0 || x > w {
                    continue;
                }
                if let Some(r) = Rect::from_xywh(x, horizon - bh, bw, bh) {
                    let mut paint = Paint::default();
                    paint.set_color(color);
                    pm.fill_rect(r, &paint, Transform::identity(), None);
                    let mut wr = Lcg((bx * 1000.0) as u64);
                    paint.set_color(window);
                    let dot = (bw * 0.12).max(1.0);
                    for _ in 0..(bh / dot / 4.0) as usize {
                        let (wx, wy) = (x + bw * (0.1 + 0.8 * wr.next()), horizon - bh * (0.1 + 0.85 * wr.next()));
                        if let Some(d) = Rect::from_xywh(wx, wy, dot, dot) {
                            pm.fill_rect(d, &paint, Transform::identity(), None);
                        }
                    }
                }
            }
            bx += bw + w * 0.004;
        }
    }

    fn paint_reflection(&self, pm: &mut Pixmap, cam: &Camera, i: usize, scale: f32) {
        let x0 = i as f32 * SPACING - PILLAR_W / 2.0;
        let depth = 0.6 * self.height_of(i, scale);
        let (Some((l, top)), Some((r, bot))) = (cam.project(x0, 0.0, 0.0), cam.project(x0 + PILLAR_W, -depth, 0.0)) else { return };
        let Some(rect) = Rect::from_ltrb(l, top, r, bot) else { return };
        let c = self.colors[i];
        let shader = LinearGradient::new(
            Point::from_xy(0.0, top),
            Point::from_xy(0.0, bot),
            vec![
                GradientStop::new(0.0, Color::from_rgba(c.red(), c.green(), c.blue(), 0.22).unwrap()),
                GradientStop::new(1.0, Color::from_rgba(c.red(), c.green(), c.blue(), 0.0).unwrap()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(Shader::SolidColor(Color::TRANSPARENT));
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        pm.fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn paint_pillar(&self, pm: &mut Pixmap, cam: &Camera, i: usize, scale: f32) {
        let x0 = i as f32 * SPACING - PILLAR_W / 2.0;
        let x1 = x0 + PILLAR_W;
        let h = self.height_of(i, scale);
        let c = self.colors[i];
        let pr = |x: f32, y: f32, z: f32| cam.project(x, y, z);
        // side facing the camera
        let sx = if (x0 + x1) / 2.0 > cam.x { x0 } else { x1 };
        if let (Some(a), Some(b), Some(cc), Some(d)) = (pr(sx, 0.0, 0.0), pr(sx, h, 0.0), pr(sx, h, PILLAR_D), pr(sx, 0.0, PILLAR_D)) {
            if let Some(q) = quad([a, b, cc, d]) {
                fill(pm, &q, shade(c, 0.68));
            }
        }
        // top, visible when below eye height
        if h < cam.eye_y {
            if let (Some(a), Some(b), Some(cc), Some(d)) = (pr(x0, h, 0.0), pr(x1, h, 0.0), pr(x1, h, PILLAR_D), pr(x0, h, PILLAR_D)) {
                if let Some(q) = quad([a, b, cc, d]) {
                    fill(pm, &q, shade(c, 1.18));
                }
            }
        }
        // front
        if let (Some((l, t)), Some((r, b))) = (pr(x0, h, 0.0), pr(x1, 0.0, 0.0)) {
            if let Some(rect) = Rect::from_ltrb(l, t, r, b) {
                let mut paint = Paint::default();
                paint.set_color(c);
                pm.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PillarColors, VideoConfig};
    use crate::pillars::Pillar;
    use tiny_skia::Pixmap;

    pub(crate) fn rows(values: &[f64]) -> Vec<Pillar> {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| Pillar {
                label: format!("{}", 1990 + i),
                name: format!("Name {i}"),
                value: v,
                detail: "Detail".into(),
                image: None,
                source: String::new(),
                estimated: false,
            })
            .collect()
    }

    fn near(px: tiny_skia::PremultipliedColorU8, c: Color, tol: i32) -> bool {
        let c = c.to_color_u8();
        (px.red() as i32 - c.red() as i32).abs() <= tol
            && (px.green() as i32 - c.green() as i32).abs() <= tol
            && (px.blue() as i32 - c.blue() as i32).abs() <= tol
    }

    fn frame(rows_: &[Pillar], cfg: &VideoConfig, p: f64) -> (WalkPainter, Pixmap, f32) {
        let mut w = WalkPainter::new(rows_, cfg, 960, 540, false).unwrap();
        let mut f = crate::camera::HeightFollow::new();
        let values: Vec<f64> = rows_.iter().map(|r| r.value).collect();
        let scale = f.step(&values, p, 1.0 / 60.0);
        let mut pm = Pixmap::new(960, 540).unwrap();
        w.paint(&mut pm, p, scale);
        (w, pm, scale)
    }

    #[test]
    fn sky_at_top_floor_at_bottom() {
        let (_, pm, _) = frame(&rows(&[5.0]), &VideoConfig::default(), 0.0);
        assert!(near(pm.pixel(480, 1).unwrap(), crate::draw::hex(SKY_TOP), 25), "{:?}", pm.pixel(480, 1));
        assert!(near(pm.pixel(958, 538).unwrap(), crate::draw::hex(FLOOR), 25), "{:?}", pm.pixel(958, 538));
    }

    #[test]
    fn focused_pillar_front_face_coloured() {
        let (w, pm, scale) = frame(&rows(&[1.0, 2.0, 3.0, 4.0, 5.0]), &VideoConfig::default(), 2.0);
        let (x, y, fw, fh) = w.pillar_rect(2, 2.0, scale).unwrap();
        let px = pm.pixel((x + fw / 2.0) as u32, (y + fh * 0.85) as u32).unwrap();
        assert!(near(px, w.pillar_color(2), 6), "{px:?}");
    }

    #[test]
    fn side_face_darker_than_front() {
        let (w, pm, scale) = frame(&rows(&[3.0, 3.0, 3.0, 3.0, 3.0]), &VideoConfig::default(), 2.0);
        let (x, y, fw, fh) = w.pillar_rect(3, 2.0, scale).unwrap();
        let lum = |p: tiny_skia::PremultipliedColorU8| p.red() as u32 + p.green() as u32 + p.blue() as u32;
        let front = pm.pixel((x + fw / 2.0) as u32, (y + fh * 0.85) as u32).unwrap();
        let side = pm.pixel((x - 3.0) as u32, (y + fh * 0.85) as u32).unwrap();
        assert!(lum(side) + 30 < lum(front), "side {side:?} front {front:?}");
    }

    #[test]
    fn reflection_below_floor_line() {
        let (w, pm, scale) = frame(&rows(&[5.0]), &VideoConfig::default(), 0.0);
        let (x, y, fw, fh) = w.pillar_rect(0, 0.0, scale).unwrap();
        let yy = (y + fh + 6.0) as u32;
        let under = pm.pixel((x + fw / 2.0) as u32, yy).unwrap();
        let plain = pm.pixel(955, yy).unwrap();
        assert!(under.red() as i32 - plain.red() as i32 > 10, "under {under:?} plain {plain:?}");
    }

    #[test]
    fn image_pillar_colour_mode() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("g.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 160, 60, 255])).save(&png).unwrap();
        let mut r = rows(&[5.0]);
        r[0].image = Some(png);
        let img_cfg = VideoConfig { pillar_colors: PillarColors::Image, ..Default::default() };
        let w = WalkPainter::new(&r, &img_cfg, 960, 540, false).unwrap();
        assert_eq!(w.pillar_color(0).to_color_u8(), Color::from_rgba8(0, 160, 60, 255).to_color_u8());
        let w = WalkPainter::new(&r, &VideoConfig::default(), 960, 540, false).unwrap();
        assert_eq!(w.pillar_color(0).to_color_u8(), crate::draw::hex("#d9822b").to_color_u8());
    }

    #[test]
    #[ignore]
    fn dev_snapshot() {
        let vals = [8000.0, 12500.0, 15000.0, 20000.0, 25000.0, 30000.0, 65000.0, 100000.0, 250000.0, 380000.0];
        for (portrait, (w, h), name) in [(false, (1920, 1080), "land"), (true, (1080, 1920), "port")] {
            let mut wp = WalkPainter::new(&rows(&vals), &VideoConfig::default(), w, h, portrait).unwrap();
            let mut f = crate::camera::HeightFollow::new();
            let mut s = 0.0;
            for _ in 0..200 { s = f.step(&vals, 5.0, 1.0 / 60.0); }
            let mut pm = Pixmap::new(w, h).unwrap();
            wp.paint(&mut pm, 5.0, s);
            pm.save_png(std::env::temp_dir().join(format!("walk_{name}.png"))).unwrap();
        }
    }

    fn fulks(image: Option<std::path::PathBuf>) -> Vec<Pillar> {
        vec![Pillar {
            label: "1946-47".into(),
            name: "Joe Fulks".into(),
            value: 8000.0,
            detail: "Philadelphia Warriors".into(),
            image,
            source: String::new(),
            estimated: false,
        }]
    }

    #[test]
    fn plate_strings_for_visible_pillars() {
        let (w, _, _) = frame(&fulks(None), &VideoConfig::default(), 0.0);
        for s in ["Joe Fulks", "8,000", "Philadelphia Warriors", "1946-47"] {
            assert!(w.last_strings.iter().any(|x| x == s), "{s}: {:?}", w.last_strings);
        }
    }

    #[test]
    fn tiny_pillar_keeps_plate_height() {
        let r = rows(&[8000.0, 40_000_000.0]);
        let mut wp = WalkPainter::new(&r, &VideoConfig::default(), 960, 540, false).unwrap();
        let mut f = crate::camera::HeightFollow::new();
        let mut s = 0.0;
        for _ in 0..200 {
            s = f.step(&[8000.0, 40_000_000.0], 1.0, 1.0 / 60.0);
        }
        let mut pm = Pixmap::new(960, 540).unwrap();
        wp.paint(&mut pm, 1.0, s);
        let (_, _, fw, fh) = wp.pillar_rect(0, 1.0, s).unwrap();
        assert!(fh >= PLATE_H * fw, "h {fh} < plate {}", PLATE_H * fw);
    }

    #[test]
    fn long_name_fits_face() {
        let short = fulks(None);
        let mut long = fulks(None);
        long[0].name = "An Extraordinarily Long Player Name ".repeat(4);
        let (w, a, s) = frame(&short, &VideoConfig::default(), 0.0);
        let (_, b, _) = frame(&long, &VideoConfig::default(), 0.0);
        let (x, y, fw, fh) = w.pillar_rect(0, 0.0, s).unwrap();
        for yy in 0..540 {
            for xx in 0..960 {
                if a.pixel(xx, yy) != b.pixel(xx, yy) {
                    let (fx, fy) = (xx as f32, yy as f32);
                    // the initials tile above the face also changes with the name
                    let top = y - fw * PHOTO_H / PILLAR_W - 2.0;
                    assert!(fx >= x - 2.0 && fx <= x + fw + 2.0 && fy >= top && fy <= y + fh + 2.0, "pixel {xx},{yy} outside face/photo rect {x},{y},{fw},{fh} top {top}");
                }
            }
        }
    }

    #[test]
    fn missing_image_initials() {
        let dir = crate::testutil::temp_week("name\n");
        let bad = dir.join("bad.jpg");
        std::fs::write(&bad, b"not an image").unwrap();
        let (w, _, _) = frame(&fulks(Some(bad)), &VideoConfig::default(), 0.0);
        assert!(w.last_strings.iter().any(|x| x == "JF"), "{:?}", w.last_strings);
    }

    #[test]
    fn photo_above_pillar() {
        let dir = crate::testutil::temp_week("name\n");
        let png = dir.join("red.png");
        image::RgbaImage::from_pixel(80, 100, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
        let (w, pm, s) = frame(&fulks(Some(png)), &VideoConfig::default(), 0.0);
        let (x, y, fw, _) = w.pillar_rect(0, 0.0, s).unwrap();
        let px = pm.pixel((x + fw / 2.0) as u32, (y - 0.3 * fw) as u32).unwrap();
        assert!(px.red() > 230 && px.green() < 30, "{px:?}");
    }

    #[test]
    fn whole_plate_values_have_no_decimal() {
        let mut r = fulks(None);
        r[0].value = 167.0;
        let cfg = VideoConfig { unit_suffix: " m".into(), ..Default::default() };
        let (w, _, _) = frame(&r, &cfg, 0.0);
        assert!(w.last_strings.iter().any(|x| x == "167 m"), "{:?}", w.last_strings);
    }

    #[test]
    #[ignore]
    fn dev_bench() {
        let vals: Vec<f64> = (0..12).map(|i| 100.0 + 60.0 * i as f64).collect();
        let r = rows(&vals);
        let mut wp = WalkPainter::new(&r, &VideoConfig::default(), 1920, 1080, false).unwrap();
        let mut pm = Pixmap::new(1920, 1080).unwrap();
        let t0 = std::time::Instant::now();
        for f in 0..120 {
            wp.paint(&mut pm, f as f64 * 0.05, 0.004);
        }
        let all = t0.elapsed();
        let cam = wp.camera(3.0);
        let t1 = std::time::Instant::now();
        for _ in 0..120 {
            vgradient(&mut pm, 0.0, cam.horizon, hex(SKY_TOP), hex(SKY_HORIZON));
            vgradient(&mut pm, cam.horizon, 1080.0, hex(FLOOR_HORIZON), hex(FLOOR));
        }
        let grad = t1.elapsed();
        let t2 = std::time::Instant::now();
        for _ in 0..120 {
            wp.paint_clouds(&mut pm, 10.0, cam.horizon);
        }
        let clouds = t2.elapsed();
        let t3 = std::time::Instant::now();
        for _ in 0..120 {
            wp.paint_skyline(&mut pm, 10.0, cam.horizon);
        }
        let sky = t3.elapsed();
        eprintln!("BENCH per frame: all {:?} grad {:?} clouds {:?} skyline {:?}", all / 120, grad / 120, clouds / 120, sky / 120);
    }
}
