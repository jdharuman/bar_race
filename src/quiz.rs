//! Flag / capital quiz Short (9:16): intro, N questions (prompt, 4 choices, 3-s countdown
//! ring, then the answer turns green with a one-line fact), outro asking for the score.
//! Input: quiz.csv (image,prompt,a,b,c,d,answer,fact) + flags/ from automation/quiz.py.

use crate::draw::{fill_path, hex, rounded_rect, with_alpha};
use crate::text::{Align, Text};
use anyhow::{bail, Context, Result};
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Point, Rect,
    SpreadMode, Stroke, Transform,
};

pub const INTRO_S: f64 = 2.4;
pub const SHOW_S: f64 = 0.45;
pub const COUNT_S: f64 = 3.0;
pub const REVEAL_S: f64 = 1.55;
pub const OUTRO_S: f64 = 3.6;
const Q_S: f64 = SHOW_S + COUNT_S + REVEAL_S;
pub const MAX_QUESTIONS: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub image: Option<PathBuf>,
    pub prompt: String,
    pub choices: [String; 4],
    /// 0-based index of the right choice.
    pub answer: usize,
    pub fact: String,
}

/// Read quiz.csv. Returns the usable questions and every problem found.
pub fn load(dir: &Path) -> Result<(Vec<Question>, Vec<String>)> {
    let path = dir.join("quiz.csv");
    let mut rdr = csv::Reader::from_path(&path).with_context(|| format!("reading {}", path.display()))?;
    let (mut qs, mut errors) = (Vec::new(), Vec::new());
    for (i, rec) in rdr.deserialize::<std::collections::HashMap<String, String>>().enumerate() {
        let line = i + 2;
        let r = match rec {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("quiz.csv line {line}: {e}"));
                continue;
            }
        };
        let get = |k: &str| r.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
        let choices = [get("a"), get("b"), get("c"), get("d")];
        let answer = get("answer").parse::<usize>().ok().filter(|n| (1..=4).contains(n));
        let Some(answer) = answer else {
            errors.push(format!("quiz.csv line {line}: answer must be 1–4 (got {:?})", get("answer")));
            continue;
        };
        if choices.iter().any(|c| c.is_empty()) {
            errors.push(format!("quiz.csv line {line}: all four choices a–d are needed"));
            continue;
        }
        let mut uniq: Vec<String> = choices.iter().map(|c| c.to_lowercase()).collect();
        uniq.sort();
        uniq.dedup();
        if uniq.len() != 4 {
            errors.push(format!("quiz.csv line {line}: the four choices must differ"));
            continue;
        }
        let image = Some(get("image")).filter(|s| !s.is_empty()).map(|s| dir.join(s));
        if let Some(p) = image.as_ref().filter(|p| !p.exists()) {
            errors.push(format!("quiz.csv line {line}: image not found: {}", p.display()));
            continue;
        }
        if image.is_none() && get("prompt").is_empty() {
            errors.push(format!("quiz.csv line {line}: needs an image or a prompt"));
            continue;
        }
        qs.push(Question { image, prompt: get("prompt"), choices, answer: answer - 1, fact: get("fact") });
    }
    if qs.len() + errors.len() == 0 {
        errors.push("quiz.csv has no questions".into());
    } else if qs.len() + errors.len() > MAX_QUESTIONS {
        errors.push(format!("quiz.csv: at most {MAX_QUESTIONS} questions (got {})", qs.len() + errors.len()));
    }
    Ok((qs, errors))
}

pub fn total_seconds(n: usize) -> f64 {
    INTRO_S + n as f64 * Q_S + OUTRO_S
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QPhase {
    Intro { t: f64 },
    /// `t` seconds into question `i` (prompt slides in, then the countdown runs).
    Ask { i: usize, t: f64 },
    /// `t` seconds after the countdown of question `i` ran out.
    Reveal { i: usize, t: f64 },
    Outro { t: f64 },
}

pub fn phase_at(sec: f64, n: usize) -> QPhase {
    if sec < INTRO_S {
        return QPhase::Intro { t: sec };
    }
    let s = sec - INTRO_S;
    let i = (s / Q_S) as usize;
    if i >= n {
        return QPhase::Outro { t: s - n as f64 * Q_S };
    }
    let t = s - i as f64 * Q_S;
    if t < SHOW_S + COUNT_S {
        QPhase::Ask { i, t }
    } else {
        QPhase::Reveal { i, t: t - SHOW_S - COUNT_S }
    }
}

const BG_TOP: &str = "#0a0f2c";
const BG_BOTTOM: &str = "#2a0f4d";
const ACCENT: &str = "#00e5ff";
const GREEN: &str = "#22c55e";
const BADGES: [&str; 4] = ["#00e5ff", "#ff2bd6", "#a3ff12", "#ff9f1c"];
const CHOICE_BG: &str = "#1b2250";

pub struct QuizPainter {
    w: f32,
    h: f32,
    text: Text,
    title: String,
    subtitle: String,
    channel: String,
    qs: Vec<Question>,
    /// Prompt image per question, rounded and scaled to fit its box.
    images: Vec<Option<Pixmap>>,
    bg: Pixmap,
}

/// Box the prompt image is fitted into: (x, y, w, h).
fn image_box(w: f32, h: f32, with_prompt: bool) -> (f32, f32, f32, f32) {
    if with_prompt {
        (0.25 * w, 0.165 * h, 0.5 * w, 0.15 * h)
    } else {
        (0.1 * w, 0.165 * h, 0.8 * w, 0.225 * h)
    }
}

fn smooth(x: f64) -> f32 {
    let x = x.clamp(0.0, 1.0) as f32;
    x * x * (3.0 - 2.0 * x)
}

fn ease_out_back(x: f64) -> f32 {
    let x = x.clamp(0.0, 1.0) as f32;
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

/// Load `path` scaled to fit (bw, bh), with rounded corners (premultiplied).
fn load_fitted(path: &Path, bw: f32, bh: f32) -> Result<Pixmap> {
    let img = image::open(path).with_context(|| format!("reading {}", path.display()))?;
    let s = (bw / img.width() as f32).min(bh / img.height() as f32);
    let (w, h) = (((img.width() as f32 * s).round() as u32).max(1), ((img.height() as f32 * s).round() as u32).max(1));
    let img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut raw = Pixmap::new(w, h).unwrap();
    for (dst, src) in raw.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        let pre = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *dst = tiny_skia::PremultipliedColorU8::from_rgba(pre(r), pre(g), pre(b), a).unwrap();
    }
    let mut mask = Mask::new(w, h).unwrap();
    let r = (w.min(h) as f32 * 0.06).max(4.0);
    if let Some(p) = rounded_rect(0.0, 0.0, w as f32, h as f32, r) {
        mask.fill_path(&p, FillRule::Winding, true, Transform::identity());
    }
    let mut out = Pixmap::new(w, h).unwrap();
    out.draw_pixmap(0, 0, raw.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
    Ok(out)
}

fn background(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    let shader = LinearGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(0.0, h as f32),
        vec![GradientStop::new(0.0, hex(BG_TOP)), GradientStop::new(1.0, hex(BG_BOTTOM))],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    let mut paint = Paint { shader, ..Default::default() };
    pm.fill_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(), &paint, Transform::identity(), None);
    // soft glows behind the prompt and the choices
    for (cx, cy, r, c) in [(0.5, 0.28, 0.55, ACCENT), (0.5, 0.75, 0.6, "#ff2bd6")] {
        let (cx, cy, r) = (cx * w as f32, cy * h as f32, r * w as f32);
        paint.shader = tiny_skia::RadialGradient::new(
            Point::from_xy(cx, cy),
            0.0,
            Point::from_xy(cx, cy),
            r,
            vec![GradientStop::new(0.0, with_alpha(hex(c), 0.16)), GradientStop::new(1.0, with_alpha(hex(c), 0.0))],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        pm.fill_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(), &paint, Transform::identity(), None);
    }
    pm
}

/// Points of an arc around (cx, cy), from angle a0 sweeping `sweep` radians.
fn arc(cx: f32, cy: f32, r: f32, a0: f32, sweep: f32) -> Option<tiny_skia::Path> {
    let steps = ((sweep.abs() / TAU * 96.0).ceil() as usize).max(2);
    let mut pb = PathBuilder::new();
    for k in 0..=steps {
        let a = a0 + sweep * k as f32 / steps as f32;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if k == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

fn stroke(pm: &mut Pixmap, path: &tiny_skia::Path, c: Color, width: f32) {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    let s = Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
    pm.stroke_path(path, &paint, &s, Transform::identity(), None);
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Color::from_rgba(l(a.red(), b.red()), l(a.green(), b.green()), l(a.blue(), b.blue()), l(a.alpha(), b.alpha())).unwrap()
}

/// Countdown colour: cyan with time left, amber in the middle, red at the end.
fn count_color(left: f32) -> Color {
    let (cyan, amber, red) = (hex(ACCENT), hex("#ffb020"), hex("#ff3b5c"));
    if left > 0.5 { mix(amber, cyan, (left - 0.5) * 2.0) } else { mix(red, amber, left * 2.0) }
}

impl QuizPainter {
    pub fn new(w: u32, h: u32, title: &str, subtitle: &str, channel: &str, qs: Vec<Question>) -> Result<QuizPainter> {
        let (fw, fh) = (w as f32, h as f32);
        let images = qs
            .iter()
            .map(|q| {
                q.image.as_ref().map(|p| {
                    let (_, _, bw, bh) = image_box(fw, fh, !q.prompt.is_empty());
                    load_fitted(p, bw, bh)
                })
                .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(QuizPainter {
            w: fw,
            h: fh,
            text: Text::new(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            channel: channel.to_string(),
            qs,
            images,
            bg: background(w, h),
        })
    }

    fn fit(&mut self, s: &str, size: f32, bold: bool, max_w: f32) -> f32 {
        let w = self.text.measure(s, size, bold);
        if w > max_w { size * max_w / w } else { size }
    }

    #[allow(clippy::too_many_arguments)]
    fn txt(&mut self, pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, bold: bool, c: Color, align: Align) {
        self.text.draw(pm, s, x, y, size, bold, c, align);
    }

    pub fn paint(&mut self, pm: &mut Pixmap, phase: QPhase) {
        pm.draw_pixmap(0, 0, self.bg.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        match phase {
            QPhase::Intro { t } => self.paint_intro(pm, t),
            QPhase::Ask { i, t } => self.paint_question(pm, i, t, None),
            QPhase::Reveal { i, t } => self.paint_question(pm, i, SHOW_S + COUNT_S, Some(t)),
            QPhase::Outro { t } => self.paint_outro(pm, t),
        }
    }

    fn paint_header(&mut self, pm: &mut Pixmap, i: usize) {
        let (w, h) = (self.w, self.h);
        let head = self.title.to_uppercase();
        let s = self.fit(&head, 0.07 * w, true, 0.84 * w);
        self.txt(pm, &head, w / 2.0, 0.07 * h, s, true, hex(ACCENT), Align::Center);
        let sub = format!("{}  ·  {} / {}", self.subtitle, i + 1, self.qs.len());
        let s = self.fit(&sub, 0.036 * w, false, 0.84 * w);
        self.txt(pm, &sub, w / 2.0, 0.105 * h, s, false, with_alpha(Color::WHITE, 0.8), Align::Center);
        // progress dots
        let n = self.qs.len();
        let gap = (0.06 * w).min(0.8 * w / n as f32);
        let x0 = w / 2.0 - gap * (n as f32 - 1.0) / 2.0;
        for k in 0..n {
            let (r, c) = match k.cmp(&i) {
                std::cmp::Ordering::Less => (0.008 * w, with_alpha(Color::WHITE, 0.85)),
                std::cmp::Ordering::Equal => (0.011 * w, hex(ACCENT)),
                std::cmp::Ordering::Greater => (0.008 * w, with_alpha(Color::WHITE, 0.25)),
            };
            if let Some(p) = PathBuilder::from_circle(x0 + gap * k as f32, 0.135 * h, r) {
                fill_path(pm, &p, c);
            }
        }
    }

    fn paint_question(&mut self, pm: &mut Pixmap, i: usize, t: f64, reveal: Option<f64>) {
        let (w, h) = (self.w, self.h);
        self.paint_header(pm, i);
        let q = self.qs[i].clone();
        let appear = smooth(t / SHOW_S);
        // prompt: image (pops in), and/or a big name
        if let Some(img) = self.images[i].as_ref() {
            let (bx, by, bw, bh) = image_box(w, h, !q.prompt.is_empty());
            let s = 0.85 + 0.15 * ease_out_back(t / SHOW_S);
            let (iw, ih) = (img.width() as f32 * s, img.height() as f32 * s);
            let (x, y) = (bx + (bw - iw) / 2.0, by + (bh - ih) / 2.0);
            let r = (iw.min(ih) * 0.06).max(4.0);
            if let Some(p) = rounded_rect(x + 0.012 * w, y + 0.016 * w, iw, ih, r) {
                fill_path(pm, &p, with_alpha(Color::BLACK, 0.35 * appear));
            }
            let paint = PixmapPaint { opacity: appear, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
            pm.draw_pixmap(0, 0, img.as_ref(), &paint, Transform::from_row(s, 0.0, 0.0, s, x, y), None);
            if let Some(p) = rounded_rect(x, y, iw, ih, r) {
                stroke(pm, &p, with_alpha(Color::WHITE, 0.55 * appear), 0.004 * w);
            }
        }
        if !q.prompt.is_empty() {
            let y = if self.images[i].is_some() { 0.36 * h } else { 0.27 * h };
            let size = if self.images[i].is_some() { 0.075 * w } else { 0.1 * w };
            let s = self.fit(&q.prompt, size, true, 0.84 * w);
            self.txt(pm, &q.prompt, w / 2.0, y, s, true, with_alpha(Color::WHITE, appear), Align::Center);
        }
        // countdown ring, replaced by the fact on reveal
        let (cx, cy, r) = (w / 2.0, 0.445 * h, 0.05 * w);
        match reveal {
            None => {
                let left = (1.0 - (t - SHOW_S) / COUNT_S).clamp(0.0, 1.0) as f32;
                let c = count_color(left);
                if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
                    fill_path(pm, &p, with_alpha(hex(CHOICE_BG), 0.9 * appear));
                    stroke(pm, &p, with_alpha(Color::WHITE, 0.15 * appear), 0.012 * w);
                }
                if let Some(p) = arc(cx, cy, r, -PI / 2.0, TAU * left).filter(|_| left > 0.0) {
                    stroke(pm, &p, with_alpha(c, appear), 0.014 * w);
                }
                let secs = ((COUNT_S as f32 * left).ceil() as i32).max(1).to_string();
                self.txt(pm, &secs, cx, cy, 0.06 * w, true, with_alpha(Color::WHITE, appear), Align::Center);
            }
            Some(rt) => {
                let a = smooth(rt / 0.3);
                if !q.fact.is_empty() {
                    let s = self.fit(&q.fact, 0.045 * w, true, 0.76 * w);
                    let tw = self.text.measure(&q.fact, s, true);
                    let (pw, ph) = (tw + 0.08 * w, s * 1.9);
                    if let Some(p) = rounded_rect(cx - pw / 2.0, cy - ph / 2.0, pw, ph, ph / 2.0) {
                        fill_path(pm, &p, with_alpha(hex(GREEN), 0.22 * a));
                        stroke(pm, &p, with_alpha(hex(GREEN), 0.9 * a), 0.004 * w);
                    }
                    self.txt(pm, &q.fact, cx, cy, s, true, with_alpha(Color::WHITE, a), Align::Center);
                }
            }
        }
        // the four choices
        let (x, bw, bh, gap, y0) = (0.1 * w, 0.8 * w, 0.068 * h, 0.016 * h, 0.5 * h);
        for k in 0..4 {
            let stagger = smooth((t - 0.06 * k as f64) / SHOW_S);
            let y = y0 + k as f32 * (bh + gap) + (1.0 - stagger) * 0.03 * h;
            let correct = k == q.answer;
            let (fill, label_a, grow) = match reveal {
                Some(rt) if correct => {
                    let g = smooth(rt / 0.25);
                    (mix(hex(CHOICE_BG), hex(GREEN), g), 1.0, 0.02 * w * (1.0 - smooth((rt - 0.25) / 0.3)) * g)
                }
                Some(rt) => (hex(CHOICE_BG), 1.0 - 0.6 * smooth(rt / 0.25), 0.0),
                None => (hex(CHOICE_BG), 1.0, 0.0),
            };
            let a = stagger * label_a;
            let (rx, ry, rw, rh) = (x - grow, y - grow / 2.0, bw + 2.0 * grow, bh + grow);
            if let Some(p) = rounded_rect(rx, ry, rw, rh, rh * 0.3) {
                fill_path(pm, &p, with_alpha(fill, 0.95 * a.max(if correct && reveal.is_some() { 1.0 } else { 0.0 })));
                stroke(pm, &p, with_alpha(hex(BADGES[k]), 0.5 * a), 0.003 * w);
            }
            let (bx, by, br) = (rx + rh * 0.55, ry + rh / 2.0, rh * 0.32);
            if let Some(p) = PathBuilder::from_circle(bx, by, br) {
                fill_path(pm, &p, with_alpha(hex(BADGES[k]), a));
            }
            let letter = ["A", "B", "C", "D"][k];
            self.txt(pm, letter, bx, by, br * 1.15, true, with_alpha(hex("#0a0f2c"), a), Align::Center);
            let tx = bx + br + 0.03 * w;
            let s = self.fit(&q.choices[k], 0.048 * w, true, rx + rw - tx - 0.03 * w);
            let c = if correct && reveal.is_some() { Color::WHITE } else { with_alpha(Color::WHITE, a) };
            self.txt(pm, &q.choices[k].clone(), tx, by, s, true, c, Align::Left);
        }
    }

    fn paint_intro(&mut self, pm: &mut Pixmap, t: f64) {
        let (w, h) = (self.w, self.h);
        let a = smooth(t / 0.35) * (1.0 - smooth((t - INTRO_S + 0.3) / 0.3));
        let pop = ease_out_back(t / 0.5);
        let head = self.title.to_uppercase();
        let s = self.fit(&head, 0.12 * w * pop.max(0.01), true, 0.9 * w);
        self.txt(pm, &head, w / 2.0, 0.36 * h, s, true, with_alpha(hex(ACCENT), a), Align::Center);
        let s = self.fit(&self.subtitle.clone(), 0.065 * w, true, 0.86 * w);
        let sub = self.subtitle.clone();
        self.txt(pm, &sub, w / 2.0, 0.43 * h, s, true, with_alpha(Color::WHITE, a), Align::Center);
        let a2 = a * smooth((t - 0.5) / 0.4);
        let line = format!("{} questions  ·  3 seconds each", self.qs.len());
        self.txt(pm, &line, w / 2.0, 0.52 * h, 0.045 * w, false, with_alpha(Color::WHITE, 0.85 * a2), Align::Center);
        self.txt(pm, "Keep score!", w / 2.0, 0.58 * h, 0.055 * w, true, with_alpha(hex("#ff2bd6"), a2), Align::Center);
    }

    fn paint_outro(&mut self, pm: &mut Pixmap, t: f64) {
        let (w, h) = (self.w, self.h);
        let a = smooth(t / 0.4);
        let pop = ease_out_back(t / 0.6);
        let s = self.fit("How many did you get?", 0.075 * w, true, 0.88 * w);
        self.txt(pm, "How many did you get?", w / 2.0, 0.33 * h, s, true, with_alpha(Color::WHITE, a), Align::Center);
        let score = format!("__ / {}", self.qs.len());
        let s = 0.16 * w * pop.max(0.01);
        self.txt(pm, &score, w / 2.0, 0.43 * h, s, true, with_alpha(hex(ACCENT), a), Align::Center);
        let a2 = smooth((t - 0.5) / 0.4);
        let s = self.fit("Comment your score below!", 0.058 * w, true, 0.88 * w);
        self.txt(pm, "Comment your score below!", w / 2.0, 0.53 * h, s, true, with_alpha(hex("#ff2bd6"), a2), Align::Center);
        let a3 = smooth((t - 1.0) / 0.4);
        self.txt(pm, "Follow for more quizzes", w / 2.0, 0.6 * h, 0.042 * w, false, with_alpha(Color::WHITE, 0.85 * a3), Align::Center);
        if !self.channel.is_empty() {
            let name = self.channel.clone();
            self.txt(pm, &name, w / 2.0, 0.68 * h, 0.05 * w, true, with_alpha(Color::WHITE, a3), Align::Center);
        }
    }
}

/// Render out/short.mp4 for a quiz folder. Quizzes are Shorts only.
pub fn render(dir: &Path, cfg: &crate::config::VideoConfig, preview: bool, audio: impl FnOnce(f64) -> Result<Option<crate::encode::AudioSpec>>) -> Result<PathBuf> {
    let (qs, errors) = load(dir)?;
    if !errors.is_empty() {
        bail!("quiz.csv has errors:\n  {}", errors.join("\n  "));
    }
    let ch = crate::config::ChannelConfig::load_or_default(Path::new("channel.toml"))?;
    let (scale, fps) = if preview { (0.5, 30) } else { (1.0, 60) };
    let (w, h) = ((1080.0 * scale) as u32, (1920.0 * scale) as u32);
    let total_s = total_seconds(qs.len());
    let n = qs.len();
    let mut painter = QuizPainter::new(w, h, &cfg.title, &cfg.subtitle, &ch.name, qs)?;
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir)?;
    let out = out_dir.join("short.mp4");
    let mut enc = crate::encode::Encoder::start(&out, w, h, fps, audio(total_s)?)?;
    let mut pm = Pixmap::new(w, h).unwrap();
    let total = (total_s * fps as f64).round() as usize;
    for f in 0..total {
        painter.paint(&mut pm, phase_at(f as f64 / fps as f64, n));
        enc.write_frame(pm.data())?;
        if total >= 10 && f % (total / 10) == 0 {
            eprintln!("short.mp4: {}%", f * 100 / total);
        }
    }
    enc.finish()?;
    eprintln!("wrote {}", out.display());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_quiz(rows: &str) -> PathBuf {
        let dir = crate::testutil::temp_week("x\n");
        std::fs::write(dir.join("quiz.csv"), format!("image,prompt,a,b,c,d,answer,fact\n{rows}")).unwrap();
        dir
    }

    fn flag(dir: &Path) {
        std::fs::create_dir_all(dir.join("flags")).unwrap();
        let mut img = image::RgbaImage::new(60, 40);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 20 { image::Rgba([0, 85, 164, 255]) } else if x < 40 { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([239, 65, 53, 255]) };
        }
        img.save(dir.join("flags/fr.png")).unwrap();
    }

    #[test]
    fn loads_good_rows_and_reports_bad_ones() {
        let dir = write_quiz(
            "flags/fr.png,,France,Italy,Chad,Peru,1,Capital: Paris\n\
             ,Kerala,Chennai,Thiruvananthapuram,Patna,Shimla,2,\n\
             ,X,A,B,C,D,5,\n\
             ,X,A,A,C,D,1,\n\
             flags/zz.png,,A,B,C,D,1,\n\
             ,,A,B,C,D,1,\n",
        );
        flag(&dir);
        let (qs, errors) = load(&dir).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].answer, 0);
        assert_eq!(qs[1].choices[qs[1].answer], "Thiruvananthapuram");
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors[0].contains("1–4") && errors[1].contains("differ") && errors[2].contains("not found"));
    }

    #[test]
    fn phases_cover_the_timeline() {
        assert_eq!(phase_at(0.0, 10), QPhase::Intro { t: 0.0 });
        assert!(matches!(phase_at(INTRO_S + 0.1, 10), QPhase::Ask { i: 0, .. }));
        assert!(matches!(phase_at(INTRO_S + SHOW_S + COUNT_S + 0.1, 10), QPhase::Reveal { i: 0, .. }));
        assert!(matches!(phase_at(INTRO_S + Q_S * 9.5, 10), QPhase::Ask { i: 9, .. }));
        assert!(matches!(phase_at(total_seconds(10) - 0.01, 10), QPhase::Outro { .. }));
        // ten questions fit a Short under a minute
        assert!(total_seconds(10) < 59.0, "{}", total_seconds(10));
    }

    fn pixel(pm: &Pixmap, x: f32, y: f32) -> [u8; 3] {
        let p = pm.pixel(x as u32, y as u32).unwrap();
        [p.red(), p.green(), p.blue()]
    }

    #[test]
    fn reveal_turns_the_answer_green() {
        let dir = write_quiz("flags/fr.png,,Italy,France,Chad,Peru,2,Capital: Paris\n");
        flag(&dir);
        let (qs, _) = load(&dir).unwrap();
        let (w, h) = (540u32, 960u32);
        let mut p = QuizPainter::new(w, h, "Guess the Flag", "Europe #1", "", qs).unwrap();
        let mut pm = Pixmap::new(w, h).unwrap();
        // second choice row, right part (no text there)
        let (x, y) = (0.85 * w as f32, (0.5 + 0.068 + 0.016 + 0.034) * h as f32);
        p.paint(&mut pm, QPhase::Ask { i: 0, t: 2.0 });
        let [r, g, b] = pixel(&pm, x, y);
        assert!(g < 120, "not green while asking: {r},{g},{b}");
        p.paint(&mut pm, QPhase::Reveal { i: 0, t: 1.0 });
        let [r, g, b] = pixel(&pm, x, y);
        assert!(g > 150 && r < 100 && b < 150, "answer not green: {r},{g},{b}");
        // the flag is drawn in its box: blue left third
        let [r, _, b] = pixel(&pm, 0.5 * w as f32 - 0.2 * w as f32, 0.28 * h as f32);
        assert!(b > 120 && r < 60, "flag not drawn");
    }

    #[test]
    fn countdown_ring_shrinks() {
        let dir = write_quiz(",Kerala,Chennai,Thiruvananthapuram,Patna,Shimla,2,\n");
        let (qs, _) = load(&dir).unwrap();
        let (w, h) = (540u32, 960u32);
        let mut p = QuizPainter::new(w, h, "Guess the Capital", "Indian States #1", "", qs).unwrap();
        let mut pm = Pixmap::new(w, h).unwrap();
        // a point on the ring's left side (≈ 3/4 of the sweep from the top, clockwise)
        let (x, y) = (w as f32 / 2.0 - 0.05 * w as f32, 0.445 * h as f32);
        p.paint(&mut pm, QPhase::Ask { i: 0, t: SHOW_S + 0.1 });
        let early = pixel(&pm, x, y);
        p.paint(&mut pm, QPhase::Ask { i: 0, t: SHOW_S + COUNT_S * 0.5 });
        let late = pixel(&pm, x, y);
        assert!(early[1] > 150, "ring arc missing early: {early:?}");
        assert!(late[1] < early[1], "ring did not shrink: {early:?} -> {late:?}");
    }
}
