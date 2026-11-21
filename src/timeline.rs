use crate::config::VideoConfig;
use crate::data::Dataset;

const MIN_SECONDS_PER_YEAR: f64 = 0.8;

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
}

impl Timeline {
    pub fn new(fmt: Format, cfg: &VideoConfig, ds: &Dataset, fps: u32) -> Timeline {
        let (mut start, end) = (ds.first, ds.last);
        let years = (end - start).max(1.0);
        let (intro, race, hold, end_card) = match fmt {
            Format::Main => (3.0, years * cfg.seconds_per_year, 3.0, 4.0),
            Format::Short => {
                let race = (cfg.short_seconds - 3.0).max(1.0);
                if race / years < MIN_SECONDS_PER_YEAR {
                    start = end - (race / MIN_SECONDS_PER_YEAR).floor();
                }
                (1.5, race, 1.5, 0.0)
            }
        };
        Timeline { fps, start, end, intro, race, hold, outro: end_card }
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
            let span = self.end - self.start; // 0 for single-year data: hold still
            return Phase::Race { t: self.start + s / self.race * span };
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
    use crate::config::VideoConfig;
    use crate::data::{Dataset, Item, Point};

    fn ds(first: i32, last: i32) -> Dataset {
        Dataset::from_items(vec![Item {
            name: "A".into(),
            category: "X".into(),
            image: None,
            points: vec![
                Point { t: first as f64, value: 1.0, estimated: false },
                Point { t: last as f64, value: 2.0, estimated: false },
            ],
            sources: vec![],
        }])
    }

    #[test]
    fn main_duration() {
        let t = Timeline::new(Format::Main, &VideoConfig::default(), &ds(1960, 2026), 60);
        assert!((t.race - 231.0).abs() < 1e-9);
        assert!((t.total_seconds() - 241.0).abs() < 1e-9);
        assert_eq!(t.frame_count(), 14460);
    }

    #[test]
    fn short_full_range_when_readable() {
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &ds(1960, 2026), 60);
        assert!((t.race - 55.0).abs() < 1e-9);
        assert_eq!(t.start, 1960.0);
        assert!((t.total_seconds() - 58.0).abs() < 1e-9);
    }

    #[test]
    fn short_trims_start_year() {
        let t = Timeline::new(Format::Short, &VideoConfig::default(), &ds(1900, 2000), 60);
        assert_eq!(t.start, 1932.0);
        assert_eq!(t.end, 2000.0);
    }

    #[test]
    fn phases_in_order() {
        let cfg = VideoConfig { seconds_per_year: 0.5, ..Default::default() };
        let t = Timeline::new(Format::Main, &cfg, &ds(2000, 2003), 30);
        assert!(matches!(t.phase_at(0), Phase::Intro { t } if t == 0.0));
        let first_race = (t.intro * 30.0).round() as u64;
        assert!(matches!(t.phase_at(first_race), Phase::Race { t } if t == 2000.0));
        let last_race = ((t.intro + t.race) * 30.0).round() as u64 - 1;
        assert!(matches!(t.phase_at(last_race), Phase::Race { t } if t <= 2003.0 && t > 2002.9));
        assert!(matches!(t.phase_at(last_race + 1), Phase::Hold));
        assert!(matches!(t.phase_at(t.frame_count() - 1), Phase::End { t } if t > 0.98));
    }

    #[test]
    fn single_year_never_races_past_end() {
        let d = ds(2000, 2000);
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
