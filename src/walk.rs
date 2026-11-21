//! Pillar walk scene: sky, clouds, skyline, glossy floor, 3D pillars with reflections.

use crate::camera::{focus_x, Camera, PILLAR_D, PILLAR_W, SPACING};
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
/// Smallest drawn pillar height (world units) — Task 5 replaces with the plate rule.
pub const MIN_H: f32 = 0.35;

pub struct WalkPainter {
    rows: Vec<Pillar>,
    colors: Vec<Color>,
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
        Ok(WalkPainter { rows: rows.to_vec(), colors, width, height, portrait, last_strings: Vec::new() })
    }

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
        vgradient(pm, 0.0, cam.horizon, hex(SKY_TOP), hex(SKY_HORIZON));
        vgradient(pm, cam.horizon, h, hex(FLOOR_HORIZON), hex(FLOOR));
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
}
