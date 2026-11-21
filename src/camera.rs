//! Perspective camera for the pillar walk. World units: pillar i stands at
//! x ∈ [i·SPACING − PILLAR_W/2, i·SPACING + PILLAR_W/2], z ∈ [0, PILLAR_D], y ∈ [0, h].

pub const SPACING: f32 = 1.6;
pub const PILLAR_W: f32 = 1.0;
pub const PILLAR_D: f32 = 0.7;
/// World height the local maximum value is scaled to.
pub const TARGET_H: f32 = 2.4;
/// World height of the photo billboard standing on a pillar (4:5 at PILLAR_W wide).
pub const PHOTO_H: f32 = 1.25;

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub x: f32,
    pub eye_y: f32,
    pub dist: f32,
    /// Focal length in pixels.
    pub focal: f32,
    /// Screen x of the optical axis.
    pub cx: f32,
    /// Screen y of the horizon (eye height at infinity).
    pub horizon: f32,
}

impl Camera {
    pub fn for_frame(width: u32, height: u32, portrait: bool) -> Camera {
        let (w, h) = (width as f32, height as f32);
        let (dist, eye_y) = (5.5, 1.1);
        let (focal, base_y) = if portrait {
            // ~3 pillars across: screen spacing = width / 2.8
            (w / 2.8 * dist / SPACING, 0.80 * h)
        } else {
            // tallest local pillar + photo spans 0.82 of the height
            (0.82 * h * dist / (TARGET_H + PHOTO_H), 0.88 * h)
        };
        Camera { x: 0.0, eye_y, dist, focal, cx: w / 2.0, horizon: base_y - focal * eye_y / dist }
    }

    /// World point → screen pixel, or None when at/behind the camera plane.
    pub fn project(&self, x: f32, y: f32, z: f32) -> Option<(f32, f32)> {
        let depth = z + self.dist;
        if depth <= 0.05 {
            return None;
        }
        Some((self.cx + self.focal * (x - self.x) / depth, self.horizon - self.focal * (y - self.eye_y) / depth))
    }
}

/// Camera x that puts pillar position `p` a little left of centre.
pub fn focus_x(p: f64, portrait: bool) -> f32 {
    p as f32 * SPACING + if portrait { 0.4 } else { 0.9 }
}

/// 0..1 → 0..1: constant speed with short quadratic ease-in/out at both ends.
pub fn ease_walk(u: f64) -> f64 {
    const A: f64 = 0.08;
    let u = u.clamp(0.0, 1.0);
    let v = 1.0 / (1.0 - A);
    if u < A {
        v * u * u / (2.0 * A)
    } else if u > 1.0 - A {
        1.0 - v * (1.0 - u) * (1.0 - u) / (2.0 * A)
    } else {
        v * (u - A / 2.0)
    }
}

/// Vertical scale (world height per value unit) that keeps nearby pillars framed.
pub struct HeightFollow {
    log_scale: Option<f64>,
}

impl HeightFollow {
    const TAU: f64 = 0.5;

    pub fn new() -> HeightFollow {
        HeightFollow { log_scale: None }
    }

    /// Smoothed so the max value among pillars p−1.5 … p+2.5 reaches TARGET_H.
    pub fn step(&mut self, values: &[f64], p: f64, dt: f64) -> f32 {
        let local_max = values
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as f64) >= p - 1.5 && (*i as f64) <= p + 2.5)
            .map(|(_, v)| *v)
            .fold(0.0, f64::max);
        if local_max <= 0.0 {
            return self.log_scale.map_or(1.0, |l| l.exp() as f32);
        }
        // smooth in log space so 5000× swings ease evenly
        let target = (TARGET_H as f64 / local_max).ln();
        let next = match self.log_scale {
            None => target,
            Some(l) => l + (target - l) * (1.0 - (-dt / Self::TAU).exp()),
        };
        self.log_scale = Some(next);
        next.exp() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_axis_point_to_centre() {
        let mut c = Camera::for_frame(1920, 1080, false);
        c.x = 5.0;
        let (sx, sy) = c.project(5.0, c.eye_y, 0.0).unwrap();
        assert!((sx - c.cx).abs() < 0.01 && (sy - c.horizon).abs() < 0.01);
    }

    #[test]
    fn farther_is_smaller() {
        let c = Camera::for_frame(1920, 1080, false);
        let w = |z: f32| c.project(1.0, 0.0, z).unwrap().0 - c.project(0.0, 0.0, z).unwrap().0;
        assert!(w(0.0) > w(3.0));
    }

    #[test]
    fn behind_camera_none() {
        let c = Camera::for_frame(1920, 1080, false);
        assert!(c.project(0.0, 0.0, -c.dist - 1.0).is_none());
    }

    #[test]
    fn ease_walk_monotone() {
        assert_eq!(ease_walk(0.0), 0.0);
        assert!((ease_walk(1.0) - 1.0).abs() < 1e-12);
        assert!((ease_walk(0.5) - 0.5).abs() < 0.01);
        let mut prev = -1.0;
        for i in 0..=100 {
            let e = ease_walk(i as f64 / 100.0);
            assert!(e > prev, "not increasing at {i}");
            prev = e;
        }
    }

    #[test]
    fn follow_targets_local_max() {
        let v = [1.0, 1.0, 1.0, 100.0, 1.0];
        let mut f = HeightFollow::new();
        let s = f.step(&v, 0.0, 1.0 / 60.0);
        assert!((s * 1.0 - TARGET_H).abs() < 1e-4, "{s}");
        let mut s = 0.0;
        for _ in 0..180 {
            s = f.step(&v, 2.0, 1.0 / 60.0);
        }
        assert!((s * 100.0 - TARGET_H).abs() / TARGET_H < 0.02, "{}", s * 100.0);
    }

    #[test]
    fn follow_handles_zero_and_nan_free() {
        let mut f = HeightFollow::new();
        assert_eq!(f.step(&[0.0, 0.0], 0.0, 1.0 / 60.0), 1.0);
        let mut f = HeightFollow::new();
        let s = f.step(&[0.0, 5e7], 0.0, 1.0 / 60.0);
        assert!(s.is_finite() && s > 0.0);
    }

    #[test]
    fn portrait_shows_fewer_pillars() {
        let count = |w: u32, h: u32, portrait: bool| {
            let mut c = Camera::for_frame(w, h, portrait);
            c.x = focus_x(10.0, portrait);
            (0..30).filter(|&i| {
                let (sx, _) = c.project(i as f32 * SPACING, 0.0, 0.0).unwrap();
                sx >= 0.0 && sx <= w as f32
            }).count()
        };
        assert!(count(1920, 1080, false) >= 4);
        assert!(count(1080, 1920, true) <= 3);
    }
}
