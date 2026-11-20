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
    pub year_size: f32,
    pub year_pos: (f32, f32),
    pub year_centered: bool,
    pub watermark_pos: (f32, f32),
    pub watermark_size: f32,
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
            year_pos: (0.95 * w, 0.84 * h),
            year_centered: false,
            watermark_pos: (0.95 * w, 0.965 * h),
            watermark_size: 0.02 * h,
        }
    }

    pub fn portrait(scale: f32, top_n: usize) -> Layout {
        let (w, h) = (1080.0 * scale, 1920.0 * scale);
        Layout {
            width: w.round() as u32,
            height: h.round() as u32,
            top_n,
            chart_left: 0.06 * w,
            chart_top: 0.20 * h,
            chart_right: 0.70 * w,
            chart_bottom: 0.70 * h,
            bar_gap: 0.2,
            title_pos: (0.06 * w, 0.085 * h),
            title_size: 0.032 * h,
            subtitle_size: 0.018 * h,
            label_size: 0.018 * h,
            year_size: 0.09 * h,
            year_pos: (0.5 * w, 0.78 * h),
            year_centered: true,
            watermark_pos: (0.94 * w, 0.865 * h),
            watermark_size: 0.013 * h,
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
}
