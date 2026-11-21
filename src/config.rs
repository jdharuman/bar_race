use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A stretch of data time played at its own pace.
#[derive(Debug, Clone, Deserialize)]
pub struct PaceSection {
    pub from: f64,
    pub to: f64,
    pub seconds_per_year: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub title: String,
    pub subtitle: String,
    pub unit_prefix: String,
    pub unit_suffix: String,
    pub top_n: usize,
    pub top_n_short: usize,
    /// Seconds per calendar year; None = default for the data's granularity.
    pub seconds_per_year: Option<f64>,
    pub pace: Vec<PaceSection>,
    pub short_seconds: f64,
    pub music: Option<PathBuf>,
    pub number_style: crate::fmt::NumberStyle,
    pub reviewed: bool,
    pub colors: BTreeMap<String, String>,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            unit_prefix: String::new(),
            unit_suffix: String::new(),
            top_n: 10,
            top_n_short: 8,
            seconds_per_year: None,
            pace: Vec::new(),
            short_seconds: 58.0,
            music: None,
            number_style: Default::default(),
            reviewed: false,
            colors: BTreeMap::new(),
        }
    }
}

impl VideoConfig {
    /// Out-of-range settings, as human-readable messages (empty = OK).
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if !(1..=20).contains(&self.top_n) {
            p.push(format!("top_n must be 1–20 (got {})", self.top_n));
        }
        if !(1..=20).contains(&self.top_n_short) {
            p.push(format!("top_n_short must be 1–20 (got {})", self.top_n_short));
        }
        let pace_ok = |s: f64| s > 0.0 && s <= 60.0;
        if let Some(s) = self.seconds_per_year.filter(|s| !pace_ok(*s)) {
            p.push(format!("seconds_per_year must be > 0 and ≤ 60 (got {s})"));
        }
        if !(8.0..=180.0).contains(&self.short_seconds) {
            p.push(format!("short_seconds must be 8–180 (got {})", self.short_seconds));
        }
        let mut secs: Vec<&PaceSection> = self.pace.iter().collect();
        secs.sort_by(|a, b| a.from.total_cmp(&b.from));
        for s in &secs {
            if !(s.from < s.to) {
                p.push(format!("[[pace]] needs from < to (got {}–{})", s.from, s.to));
            }
            if !pace_ok(s.seconds_per_year) {
                p.push(format!("[[pace]] seconds_per_year must be > 0 and ≤ 60 (got {})", s.seconds_per_year));
            }
        }
        for w in secs.windows(2) {
            if w[0].to > w[1].from {
                p.push(format!("[[pace]] sections overlap: {}–{} and {}–{}", w[0].from, w[0].to, w[1].from, w[1].to));
            }
        }
        p
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: VideoConfig = toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        let p = cfg.problems();
        if !p.is_empty() {
            anyhow::bail!("{}: {}", path.display(), p.join("; "));
        }
        Ok(cfg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    /// Watermark / end-card name; empty = none.
    pub name: String,
    pub logo: Option<PathBuf>,
    pub theme: Theme,
    /// Overrides the theme's background colour.
    pub background: Option<String>,
    /// Overrides the theme's text colour.
    pub text: Option<String>,
    pub accent: String,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            logo: None,
            theme: Theme::Light,
            background: None,
            text: None,
            accent: "#38bdf8".into(),
        }
    }
}

impl ChannelConfig {
    /// (background, text) hex colours after theme defaults and overrides.
    pub fn colors(&self) -> (String, String) {
        let (bg, fg) = match self.theme {
            Theme::Light => ("#ffffff", "#1f2937"),
            Theme::Dark => ("#0f172a", "#ffffff"),
        };
        (self.background.clone().unwrap_or(bg.into()), self.text.clone().unwrap_or(fg.into()))
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(path)?;
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(VideoConfig::default().problems().is_empty());
    }

    #[test]
    fn out_of_range_values_reported() {
        let bad = VideoConfig { top_n: 0, top_n_short: 50, seconds_per_year: Some(-1.0), short_seconds: 3.0, ..Default::default() };
        let p = bad.problems().join("\n");
        for key in ["top_n ", "top_n_short", "seconds_per_year", "short_seconds"] {
            assert!(p.contains(key), "missing {key} in {p}");
        }
    }

    #[test]
    fn overlapping_sections_rejected() {
        let c = VideoConfig {
            pace: vec![
                PaceSection { from: 1600.0, to: 1800.0, seconds_per_year: 0.45 },
                PaceSection { from: 1700.0, to: 1900.0, seconds_per_year: 1.0 },
            ],
            ..Default::default()
        };
        assert!(c.problems().join(" ").contains("overlap"), "{:?}", c.problems());
    }

    #[test]
    fn backwards_section_rejected() {
        let c = VideoConfig { pace: vec![PaceSection { from: 1800.0, to: 1800.0, seconds_per_year: 1.0 }], ..Default::default() };
        assert!(c.problems().join(" ").contains("from < to"), "{:?}", c.problems());
    }

    #[test]
    fn pace_toml_accepts_integers() {
        let c: VideoConfig = toml::from_str("[[pace]]\nfrom = 1600\nto = 1800\nseconds_per_year = 0.45\n").unwrap();
        assert_eq!(c.pace[0].from, 1600.0);
        assert_eq!(c.seconds_per_year, None);
    }
}
