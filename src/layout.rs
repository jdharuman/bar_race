/// Pixel geometry for one output format. `scale` 1.0 = full resolution, 0.5 = preview.
#[derive(Debug, Clone)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub top_n: usize,
    pub chart_left: f32,
    pub chart_top: f32,
    pub chart_right: f32,
    pub chart_bottom: f32,
    /// Fraction of each slot left empty between bars.
    pub bar_gap: f32,
    pub title_pos: (f32, f32),
    pub title_size: f32,
    pub subtitle_size: f32,
    pub label_size: f32,
    /// Largest year-counter size; shrinks to fit `year_box`.
    pub year_size: f32,
    /// 9:16 Short layout (overlays below the chart) vs 16:9 (overlays lower right).
    pub portrait: bool,
    pub watermark_pos: (f32, f32),
    pub watermark_size: f32,
    /// Overrides `caption_box` (the line chart puts captions elsewhere).
    pub caption_at: Option<(f32, f32, f32, f32)>,
}

impl Layout {
    pub fn landscape(scale: f32, top_n: usize) -> Layout {
        let (w, h) = (1920.0 * scale, 1080.0 * scale);
        Layout {
            width: w.round() as u32,
            height: h.round() as u32,
            top_n,
            chart_left: 0.05 * w,
            chart_top: 0.20 * h,
            chart_right: 0.78 * w,
            chart_bottom: 0.93 * h,
            bar_gap: 0.2,
            title_pos: (0.05 * w, 0.075 * h),
            title_size: 0.048 * h,
            subtitle_size: 0.026 * h,
            label_size: 0.026 * h,
            year_size: 0.17 * h,
            portrait: false,
            watermark_pos: (0.95 * w, 0.965 * h),
            watermark_size: 0.02 * h,
            caption_at: None,
        }
    }

    pub fn portrait(scale: f32, top_n: usize) -> Layout {
        let (w, h) = (1080.0 * scale, 1920.0 * scale);
        Layout {
            width: w.round() as u32,
            height: h.round() as u32,
            top_n,
            chart_left: 0.06 * w,
            chart_top: 0.16 * h,
            chart_right: 0.58 * w,
            chart_bottom: 0.87 * h,
            bar_gap: 0.2,
            title_pos: (0.06 * w, 0.065 * h),
            title_size: 0.032 * h,
            subtitle_size: 0.018 * h,
            label_size: 0.0165 * h,
            year_size: 0.09 * h,
            portrait: true,
            watermark_pos: (0.94 * w, 0.865 * h),
            watermark_size: 0.013 * h,
            caption_at: None,
        }
    }

    pub fn slot_height(&self) -> f32 {
        (self.chart_bottom - self.chart_top) / self.top_n as f32
    }

    pub fn bar_height(&self) -> f32 {
        self.slot_height() * (1.0 - self.bar_gap)
    }

    pub fn slot_center_y(&self, slot: f32) -> f32 {
        self.chart_top + (slot + 0.5) * self.slot_height()
    }

    fn wh(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }

    /// Fact caption area (x, y, w, h).
    pub fn caption_box(&self) -> (f32, f32, f32, f32) {
        if let Some(b) = self.caption_at {
            return b;
        }
        let (w, h) = self.wh();
        if self.portrait {
            (0.60 * w, 0.50 * h, 0.36 * w, 0.11 * h)
        } else {
            (0.60 * w, 0.43 * h, 0.37 * w, 0.17 * h)
        }
    }

    /// Leader badge area (square).
    pub fn badge_box(&self) -> (f32, f32, f32, f32) {
        let (w, h) = self.wh();
        if self.portrait {
            let s = 0.085 * h;
            (0.94 * w - s, 0.61 * h, s, s)
        } else {
            let s = 0.12 * h;
            (0.95 * w - s, 0.61 * h, s, s)
        }
    }

    /// Year counter area.
    pub fn year_box(&self) -> (f32, f32, f32, f32) {
        let (w, h) = self.wh();
        if self.portrait {
            (0.60 * w, 0.715 * h, 0.36 * w, 0.09 * h)
        } else {
            (0.60 * w, 0.74 * h, 0.35 * w, 0.19 * h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type B = (f32, f32, f32, f32);
    fn inter(a: B, b: B) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    #[test]
    fn portrait_bars_use_full_height() {
        let l = Layout::portrait(1.0, 12);
        assert!(l.chart_bottom >= 0.86 * l.height as f32, "chart ends at {}", l.chart_bottom / l.height as f32);
        assert!(l.chart_bottom <= 0.88 * l.height as f32);
    }

    #[test]
    fn overlay_boxes_clear_of_each_other_and_top_bars() {
        for l in [Layout::landscape(1.0, 10), Layout::portrait(1.0, 12)] {
            let boxes = [l.caption_box(), l.badge_box(), l.year_box()];
            for b in boxes {
                assert!(b.0 >= 0.0 && b.1 >= 0.0 && b.0 + b.2 <= l.width as f32 && b.1 + b.3 <= l.height as f32, "{b:?}");
                // both formats: overlays in the lower right, clear of the top (longest) bars
                let top_slots = if l.portrait { 4.0 } else { 3.0 };
                assert!(b.1 >= l.chart_top + top_slots * l.slot_height(), "{b:?}");
                assert!(b.0 >= (l.chart_left + l.chart_right) / 2.0, "{b:?}");
                if l.portrait {
                    assert!(b.1 + b.3 <= 0.88 * l.height as f32, "Shorts UI zone: {b:?}");
                }
            }
            for i in 0..3 {
                for j in i + 1..3 {
                    assert!(!inter(boxes[i], boxes[j]), "{i} vs {j}: {:?} {:?}", boxes[i], boxes[j]);
                }
            }
        }
    }
}
