use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use std::collections::HashMap;
use tiny_skia::{Color, Pixmap, PremultipliedColorU8};

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
// ponytail: cache is cleared wholesale when full; LRU if label churn ever shows in profiles.
const CACHE_LIMIT: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Right,
    Center,
}

pub struct Text {
    fs: FontSystem,
    cache: SwashCache,
    buffers: HashMap<(String, u32, bool), Buffer>,
}

impl Text {
    pub fn new() -> Text {
        let mut fs = FontSystem::new();
        fs.db_mut().load_font_data(INTER_REGULAR.to_vec());
        fs.db_mut().load_font_data(INTER_BOLD.to_vec());
        Text { fs, cache: SwashCache::new(), buffers: HashMap::new() }
    }

    fn shaped(&mut self, s: &str, size: f32, bold: bool) -> (&mut FontSystem, &mut SwashCache, &mut Buffer) {
        let Text { fs, cache, buffers } = self;
        if buffers.len() > CACHE_LIMIT {
            buffers.clear();
        }
        let key = (s.to_string(), (size * 10.0) as u32, bold);
        let buf = buffers.entry(key).or_insert_with(|| {
            let mut b = Buffer::new(fs, Metrics::new(size, size * 1.2));
            b.set_size(None, None);
            let attrs = Attrs::new()
                .family(Family::Name("Inter"))
                .weight(if bold { Weight::BOLD } else { Weight::NORMAL });
            b.set_text(s, &attrs, Shaping::Advanced, None);
            b.shape_until_scroll(fs, false);
            b
        });
        (fs, cache, buf)
    }

    /// Width in pixels of `s` set at `size`.
    pub fn measure(&mut self, s: &str, size: f32, bold: bool) -> f32 {
        let (_, _, buf) = self.shaped(s, size, bold);
        buf.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    /// Draw `s` with its line box vertically centred on `y`; `x` is the left/right/centre edge per `align`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, color: Color, align: Align) {
        let w = self.measure(s, size, bold);
        let x0 = match align {
            Align::Left => x,
            Align::Right => x - w,
            Align::Center => x - w / 2.0,
        }
        .round() as i32;
        let y0 = (y - size * 1.2 / 2.0).round() as i32;
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let pixels = pm.pixels_mut();
        let (fs, cache, buf) = self.shaped(s, size, bold);
        buf.draw(fs, cache, cosmic_text::Color::rgba(255, 255, 255, 255), |gx, gy, gw, gh, c| {
            let a = c.a() as f32 / 255.0 * color.alpha();
            if a <= 0.0 {
                return;
            }
            for dy in 0..gh as i32 {
                for dx in 0..gw as i32 {
                    let (px, py) = (x0 + gx + dx, y0 + gy + dy);
                    if px >= 0 && py >= 0 && px < pw && py < ph {
                        let p = &mut pixels[(py * pw + px) as usize];
                        *p = blend(*p, color, a);
                    }
                }
            }
        });
    }
}

/// Source-over blend of straight `color` at coverage `a` onto a premultiplied pixel.
pub fn blend(dst: PremultipliedColorU8, color: Color, a: f32) -> PremultipliedColorU8 {
    let inv = 1.0 - a;
    let ch = |s: f32, d: u8| ((s * a * 255.0) + d as f32 * inv).round().clamp(0.0, 255.0) as u8;
    let na = ch(1.0, dst.alpha());
    let (r, g, b) = (ch(color.red(), dst.red()), ch(color.green(), dst.green()), ch(color.blue(), dst.blue()));
    PremultipliedColorU8::from_rgba(r.min(na), g.min(na), b.min(na), na).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Color, Pixmap};

    #[test]
    fn non_latin_names_draw() {
        let mut pm = Pixmap::new(800, 100).unwrap();
        pm.fill(Color::BLACK);
        let mut t = Text::new();
        t.draw(&mut pm, "中国 São Tomé Côte d'Ivoire", 10.0, 50.0, 40.0, false, Color::WHITE, Align::Left);
        let lit = pm.pixels().iter().filter(|p| p.red() > 128).count();
        assert!(lit >= 50, "only {lit} lit pixels");
        // CJK glyphs come from fallback: something lit in the first 90px (where 中国 sits)
        let cjk = (0..100u32)
            .flat_map(|y| (10..90u32).map(move |x| (x, y)))
            .filter(|&(x, y)| pm.pixel(x, y).unwrap().red() > 128)
            .count();
        assert!(cjk >= 30, "CJK not drawn ({cjk} px)");
    }

    #[test]
    fn measure_and_right_align() {
        let mut t = Text::new();
        let w = t.measure("Hello", 40.0, true);
        assert!(w > 60.0 && w < 200.0, "width {w}");
        let mut pm = Pixmap::new(400, 100).unwrap();
        pm.fill(Color::BLACK);
        t.draw(&mut pm, "Hello", 390.0, 50.0, 40.0, true, Color::WHITE, Align::Right);
        let max_x = (0..400u32)
            .filter(|&x| (0..100u32).any(|y| pm.pixel(x, y).unwrap().red() > 128))
            .max()
            .unwrap();
        assert!(max_x <= 392 && max_x >= 370, "right edge {max_x}");
    }
}
