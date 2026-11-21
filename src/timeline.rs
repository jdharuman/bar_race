use crate::config::VideoConfig;
use crate::data::{Dataset, Granularity};

/// Slowest-allowed Short pace (seconds per year) before its start is trimmed.
fn short_floor(g: Granularity) -> f64 {
    if g == Granularity::Year { 0.8 } else { 3.0 }
}

/// Default seconds per calendar year: 2 s per step yearly, 1.5 s per quarter, 1 s per month.
pub fn default_pace(g: Granularity) -> f64 {
    match g {
        Granularity::Year => 2.0,
        Granularity::Quarter => 6.0,
        Granularity::Month => 12.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Main,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Intro { t: f64 },
    Race { t: f64 },
    Hold,
    End { t: f64 },
}

#[derive(Debug, Clone)]
pub struct Timeline {
    pub fps: u32,
    pub start: f64,
    pub end: f64,
    pub intro: f64,
    pub race: f64,
    pub hold: f64,
    pub outro: f64,
    /// Race pieces: (t0, t1, seconds) in order.
    pub segs: Vec<(f64, f64, f64)>,
}

/// Cut [start, end] at pace-section edges; each piece gets its section's pace or `base`.
fn build_segs(start: f64, end: f64, base: f64, cfg: &VideoConfig) -> Vec<(f64, f64, f64)> {
    let mut cuts = vec![start, end];
    for s in &cfg.pace {
        cuts.extend([s.from, s.to].into_iter().filter(|&c| c > start && c < end));
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    cuts.windows(2)
        .map(|w| {
            let mid = (w[0] + w[1]) / 2.0;
            let rate = cfg.pace.iter().find(|s| s.from <= mid && mid < s.to).map_or(base, |s| s.seconds_per_year);
            (w[0], w[1], (w[1] - w[0]) * rate)
        })
        .collect()
}

impl Timeline {
    pub fn new(fmt: Format, cfg: &VideoConfig, ds: &Dataset, fps: u32) -> Timeline {
        let (mut start, end) = (ds.first, ds.last);
        let base = cfg.seconds_per_year.unwrap_or(default_pace(ds.granularity));
        let (intro, hold, outro) = match fmt {
            Format::Main => (3.0, 3.0, 5.0),
            Format::Short => (1.5, 1.0, 2.5),
        };
        let mut segs = build_segs(start, end, base, cfg);
        let race = match fmt {
            Format::Main if segs.is_empty() => base, // single-year data
            Format::Main => segs.iter().map(|s| s.2).sum(),
            Format::Short => {
                let race = cfg.short_seconds - intro - hold - outro;
                let floor = short_floor(ds.granularity);
                if end > start && race / (end - start) < floor {
                    start = (end - race / floor).ceil();
                    segs = build_segs(start, end, base, cfg);
                }
                let k = race / segs.iter().map(|s| s.2).sum::<f64>().max(1e-9);
                segs.iter_mut().for_each(|s| s.2 *= k);
                race
            }
        };
        if segs.is_empty() {
            segs = vec![(end, end, race)]; // single-year data: hold still for the race
        }
        Timeline { fps, start, end, intro, race, hold, outro, segs }
    }

    pub fn total_seconds(&self) -> f64 {
        self.intro + self.race + self.hold + self.outro
    }

    pub fn frame_count(&self) -> u64 {
        (self.total_seconds() * self.fps as f64).round() as u64
    }

    pub fn phase_at(&self, frame: u64) -> Phase {
        let s = frame as f64 / self.fps as f64;
        if s < self.intro {
            return Phase::Intro { t: s / self.intro };
        }
        let s = s - self.intro;
        if s < self.race {
            let mut acc = 0.0;
            for &(t0, t1, secs) in &self.segs {
                if s < acc + secs {
                    return Phase::Race { t: t0 + (s - acc) / secs * (t1 - t0) };
                }
                acc += secs;
            }
            return Phase::Race { t: self.end };
        }
        let s = s - self.race;
        if s < self.hold || self.outro == 0.0 {
            return Phase::Hold;
        }
        Phase::End { t: ((s - self.hold) / self.outro).min(1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PaceSection, VideoConfig};
    use crate::data::{Dataset, Granularity, Item, Point};

    fn ds(first: f64, last: f64) -> Dataset {
        Dataset::from_items(vec![Item {
            name: "A".into(),
            category: "X".into(),
            image: None,
            points: vec![Point { t: first, value: 1.0, estimated: false }, Point { t: last, value: 2.0, estimated: false }],
            sources: vec![],
        }])
    }

    fn race_t_at(t: &Timeline, secs_into_race: f64) -> f64 {
        let f = ((t.intro + secs_into_race) * t.fps as f64).round() as u64;
        match t.phase_at(f) {
            Phase::Race { t } => t,
            p => panic!("not racing: {p:?}"),
        }
    }

    #[test]
    fn main_duration() {
        let cfg = VideoConfig { seconds_per_year: Some(3.5), ..Default::default() };
        let t = Timeline::new(Format::Main, &cfg, &ds(1960.0, 2026.0), 60);
        assert!((t.race - 231.0).abs() < 1e-9);
        assert!((t.total_seconds() - 242.0).abs() < 1e-9);
        assert_eq!(t.frame_count(), 14520);
    }

    #[test]
    fn default_pace_by_granularity() {
        let t = Timeline::new(Format::Main, &VideoConfig::default(), &ds(1960.0, 2026.0), 60);
        assert!((t.race - 132.0).abs() < 1e-9);
        assert!((t.total_seconds() - 143.0).abs() < 1e-9);
        assert_eq!(default_pace(Granularity::Quarter), 6.0);
        assert_eq!(default_pace(Granularity::Month), 12.0);
    }

    fn sections(cfg_base: f64, secs: &[(f64, f64, f64)]) -> VideoConfig {
        VideoConfig {
            seconds_per_year: Some(cfg_base),
            pace: secs.iter().map(|&(from, to, s)| PaceSection { from, to, seconds_per_year: s }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn pace_sections_apply() {
        let cfg = sections(1.7, &[(1600.0, 1800.0, 0.45)]);
        let t = Timeline::new(Format::Main, &cfg, &ds(1600.0, 2000.0), 60);
        assert!((t.race - 430.0).abs() < 1e-9, "{}", t.race);
        assert!((race_t_at(&t, 90.0) - 1800.0).abs() < 0.02);
        assert!((race_t_at(&t, 45.0) - 1700.0).abs() < 0.05);
        assert!((race_t_at(&t, 90.0 + 170.0) - 1900.0).abs() < 0.02);
    }

    #[test]
    fn sections_outside_data_ignored() {
        let cfg = sections(1.7, &[(1500.0, 1550.0, 0.45)]);
        let t = Timeline::new(Format::Main, &cfg, &ds(1600.0, 2000.0), 60);
        assert!((t.race - 680.0).abs() < 1e-9);
    }

    #[test]
    fn short_scales_sections_and_has_outro() {
        let cfg = VideoConfig { short_seconds: 180.0, ..sections(1.7, &[(1700.0, 1750.0, 0.45)]) };
        let t = Timeline::new(Format::Short, &cfg, &ds(1700.0, 1800.0), 60);
        assert!((t.race - 175.0).abs() < 1e-9);
        assert_eq!(t.outro, 2.5);
        assert!((t.total_seconds() - 180.0).abs() < 1e-9);
        assert_eq!(t.start, 1700.0);
        // section share keeps its 22.5 : 85 ratio
        let split = 175.0 * 22.5 / 107.5;
        assert!((race_t_at(&t, split) - 1750.0).abs() < 0.05);
    }

    #[test]
    fn short_full_range_when_readable() {
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &ds(1960.0, 2026.0), 60);
        assert!((t.race - 53.0).abs() < 1e-9);
        assert_eq!(t.start, 1960.0);
        assert!((t.total_seconds() - 58.0).abs() < 1e-9);
    }

    #[test]
    fn short_floor_trims_to_whole_year() {
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &ds(1900.0, 2000.0), 60);
        assert_eq!(t.start, 1934.0);
        assert_eq!(t.end, 2000.0);
    }

    #[test]
    fn short_floor_subyear() {
        let mut d = ds(1996.0, 2026.0);
        d.granularity = Granularity::Month;
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &d, 60);
        assert_eq!(t.start, 2009.0);
    }

    #[test]
    fn phases_in_order() {
        let cfg = VideoConfig { seconds_per_year: Some(0.5), ..Default::default() };
        let t = Timeline::new(Format::Main, &cfg, &ds(2000.0, 2003.0), 30);
        assert!(matches!(t.phase_at(0), Phase::Intro { t } if t == 0.0));
        let first_race = (t.intro * 30.0).round() as u64;
        assert!(matches!(t.phase_at(first_race), Phase::Race { t } if t == 2000.0));
        let last_race = ((t.intro + t.race) * 30.0).round() as u64 - 1;
        assert!(matches!(t.phase_at(last_race), Phase::Race { t } if t <= 2003.0 && t > 2002.9));
        assert!(matches!(t.phase_at(last_race + 1), Phase::Hold));
        assert!(matches!(t.phase_at(t.frame_count() - 1), Phase::End { t } if t > 0.98));
    }

    #[test]
    fn short_has_end_card() {
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &ds(2000.0, 2003.0), 30);
        assert!(matches!(t.phase_at(t.frame_count() - 1), Phase::End { .. }));
    }

    #[test]
    fn single_year_never_races_past_end() {
        let d = ds(2000.0, 2000.0);
        for fmt in [Format::Main, Format::Short] {
            let t = Timeline::new(fmt, &VideoConfig::default(), &d, 30);
            for f in 0..t.frame_count() {
                if let Phase::Race { t: y } = t.phase_at(f) {
                    assert_eq!(y, 2000.0);
                }
            }
        }
    }
}
