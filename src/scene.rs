use crate::data::{Dataset, Item};

const SLOT_TAU: f64 = 0.12;
const AXIS_TAU: f64 = 0.25;
const ALPHA_TAU: f64 = 0.15;

/// Interpolated value of `item` at fractional `year`, plus whether it leans on an estimate.
/// Ramps from 0 over the year before the first point and back to 0 over the year after the last.
pub fn value_at(item: &Item, year: f64) -> Option<(f64, bool)> {
    let pts = &item.points;
    let (first, last) = (pts.first()?, pts.last()?);
    let (fy, ly) = (first.year as f64, last.year as f64);
    if year < fy - 1.0 || year > ly + 1.0 {
        return None;
    }
    if year < fy {
        return Some((first.value * (year - (fy - 1.0)), first.estimated));
    }
    if year > ly {
        return Some((last.value * (ly + 1.0 - year), last.estimated));
    }
    let i = pts.partition_point(|p| (p.year as f64) <= year);
    if i == pts.len() {
        return Some((last.value, last.estimated));
    }
    let (a, b) = (&pts[i - 1], &pts[i]);
    let t = (year - a.year as f64) / (b.year - a.year) as f64;
    Some((a.value + (b.value - a.value) * t, a.estimated || b.estimated))
}

#[derive(Debug, Clone)]
pub struct BarState {
    pub idx: usize,
    pub value: f64,
    pub slot: f32,
    pub alpha: f32,
    pub estimated: bool,
}

#[derive(Debug, Clone)]
pub struct FrameState {
    pub year: f64,
    pub bars: Vec<BarState>,
    pub axis_max: f64,
}

pub struct RankTracker {
    slot: Vec<f32>,
    alpha: Vec<f32>,
    axis_max: f64,
    top_n: usize,
    started: bool,
}

fn ease(x: f64, target: f64, dt: f64, tau: f64) -> f64 {
    x + (target - x) * (1.0 - (-dt / tau).exp())
}

impl RankTracker {
    pub fn new(n_items: usize, top_n: usize) -> RankTracker {
        RankTracker {
            slot: vec![top_n as f32; n_items],
            alpha: vec![0.0; n_items],
            axis_max: 1.0,
            top_n,
            started: false,
        }
    }

    pub fn step(&mut self, ds: &Dataset, year: f64, dt: f64) -> FrameState {
        let vals: Vec<Option<(f64, bool)>> = ds.items.iter().map(|it| value_at(it, year)).collect();
        let mut order: Vec<usize> = (0..ds.items.len()).filter(|&i| vals[i].is_some()).collect();
        order.sort_by(|&a, &b| {
            let (va, vb) = (vals[a].unwrap().0, vals[b].unwrap().0);
            vb.total_cmp(&va).then_with(|| ds.items[a].name.cmp(&ds.items[b].name))
        });
        let mut target_slot = vec![self.top_n as f32; ds.items.len()];
        let mut target_alpha = vec![0.0f32; ds.items.len()];
        for (rank, &i) in order.iter().enumerate().take(self.top_n) {
            target_slot[i] = rank as f32;
            target_alpha[i] = 1.0;
        }
        let leader = order.first().map(|&i| vals[i].unwrap().0).unwrap_or(0.0).max(1e-9);
        if !self.started {
            self.slot = target_slot;
            self.alpha = target_alpha;
            self.axis_max = leader;
            self.started = true;
        } else {
            for i in 0..ds.items.len() {
                self.slot[i] = ease(self.slot[i] as f64, target_slot[i] as f64, dt, SLOT_TAU) as f32;
                self.alpha[i] = ease(self.alpha[i] as f64, target_alpha[i] as f64, dt, ALPHA_TAU) as f32;
            }
            self.axis_max = ease(self.axis_max, leader, dt, AXIS_TAU);
        }
        let mut bars: Vec<BarState> = (0..ds.items.len())
            .filter(|&i| self.alpha[i] > 0.01)
            .map(|i| {
                let (value, estimated) = vals[i].unwrap_or((0.0, false));
                BarState { idx: i, value, slot: self.slot[i], alpha: self.alpha[i], estimated }
            })
            .collect();
        bars.sort_by(|a, b| a.slot.total_cmp(&b.slot));
        FrameState { year, bars, axis_max: self.axis_max }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Dataset, Item, Point};

    fn item(name: &str, pts: &[(i32, f64, bool)]) -> Item {
        Item {
            name: name.into(),
            category: "X".into(),
            image: None,
            points: pts.iter().map(|&(year, value, estimated)| Point { year, value, estimated }).collect(),
            sources: vec![],
        }
    }

    #[test]
    fn interpolates_linearly() {
        let a = item("A", &[(2000, 10.0, false), (2001, 20.0, false)]);
        assert_eq!(value_at(&a, 2000.5), Some((15.0, false)));
    }

    #[test]
    fn ramps_in_and_out() {
        let d = item("D", &[(2002, 8.0, false)]);
        assert_eq!(value_at(&d, 2001.5).map(|v| v.0), Some(4.0));
        assert_eq!(value_at(&d, 2002.5).map(|v| v.0), Some(4.0));
        assert_eq!(value_at(&d, 2000.9), None);
        assert_eq!(value_at(&d, 2003.1), None);
    }

    #[test]
    fn gap_interpolated_across() {
        let a = item("A", &[(2000, 0.0, false), (2003, 30.0, false)]);
        assert_eq!(value_at(&a, 2001.0).map(|v| v.0), Some(10.0));
    }

    #[test]
    fn estimated_propagates() {
        let b = item("B", &[(2000, 1.0, false), (2001, 2.0, true)]);
        assert_eq!(value_at(&b, 2000.5).map(|v| v.1), Some(true));
    }

    fn slots_order(fs: &FrameState, ds: &Dataset) -> Vec<String> {
        let mut bars = fs.bars.clone();
        bars.sort_by(|a, b| a.slot.partial_cmp(&b.slot).unwrap());
        bars.iter().map(|b| ds.items[b.idx].name.clone()).collect()
    }

    #[test]
    fn ties_stable_by_name() {
        let ds = Dataset::from_items(vec![
            item("Beta", &[(2000, 5.0, false), (2010, 50.0, false)]),
            item("Alpha", &[(2000, 5.0, false), (2010, 50.0, false)]),
        ]);
        let mut t = RankTracker::new(ds.items.len(), 10);
        for f in 0..100 {
            let fs = t.step(&ds, 2000.0 + f as f64 * 0.1, 1.0 / 60.0);
            if f > 30 {
                assert_eq!(slots_order(&fs, &ds), vec!["Alpha", "Beta"]);
            }
        }
    }

    #[test]
    fn slot_eases_not_jumps() {
        let ds = Dataset::from_items(vec![
            item("A", &[(2000, 10.0, false), (2001, 10.0, false)]),
            item("B", &[(2000, 5.0, false), (2001, 20.0, false)]),
        ]);
        let mut t = RankTracker::new(2, 10);
        for _ in 0..120 {
            t.step(&ds, 2000.0, 1.0 / 60.0);
        }
        let before = t.step(&ds, 2000.0, 1.0 / 60.0);
        let b_before = before.bars.iter().find(|b| b.idx == 1).unwrap().slot;
        assert!((b_before - 1.0).abs() < 0.05);
        let after = t.step(&ds, 2001.0, 1.0 / 60.0);
        let b_after = after.bars.iter().find(|b| b.idx == 1).unwrap().slot;
        assert!((b_before - b_after).abs() < 0.5, "jumped {b_before} → {b_after}");
        let mut last = after;
        for _ in 0..36 {
            last = t.step(&ds, 2001.0, 1.0 / 60.0);
        }
        let b = last.bars.iter().find(|b| b.idx == 1).unwrap().slot;
        assert!(b.abs() < 0.05, "B should be near top slot, got {b}");
    }

    #[test]
    fn axis_max_tracks_leader() {
        let ds = Dataset::from_items(vec![item("A", &[(2000, 100.0, false), (2001, 100.0, false)])]);
        let mut t = RankTracker::new(1, 10);
        let mut fs = t.step(&ds, 2000.5, 1.0 / 60.0);
        for _ in 0..60 {
            fs = t.step(&ds, 2000.5, 1.0 / 60.0);
        }
        assert!((fs.axis_max - 100.0).abs() / 100.0 < 0.02, "axis_max {}", fs.axis_max);
    }

    #[test]
    fn dropped_out_bar_fades() {
        let ds = Dataset::from_items(vec![
            item("A", &[(2000, 10.0, false), (2003, 10.0, false)]),
            item("Gone", &[(2000, 50.0, false)]),
        ]);
        let mut t = RankTracker::new(2, 10);
        let mut fs = t.step(&ds, 2000.0, 1.0 / 60.0);
        for _ in 0..120 {
            fs = t.step(&ds, 2002.0, 1.0 / 60.0);
        }
        assert!(fs.bars.iter().all(|b| b.idx != 1), "Gone should have left the frame");
    }
}
