use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub title: String,
    pub subtitle: String,
    pub unit_prefix: String,
    pub unit_suffix: String,
    pub top_n: usize,
    pub top_n_short: usize,
    pub seconds_per_year: f64,
    pub short_seconds: f64,
    pub music: Option<PathBuf>,
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
            seconds_per_year: 3.5,
            short_seconds: 58.0,
            music: None,
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
        if !(self.seconds_per_year > 0.0 && self.seconds_per_year <= 60.0) {
            p.push(format!("seconds_per_year must be > 0 and ≤ 60 (got {})", self.seconds_per_year));
        }
        if !(self.short_seconds > 5.0 && self.short_seconds <= 180.0) {
            p.push(format!("short_seconds must be > 5 and ≤ 180 (got {})", self.short_seconds));
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    pub name: String,
    pub logo: Option<PathBuf>,
    pub background: String,
    pub text: String,
    pub accent: String,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            name: "Stat Race".into(),
            logo: None,
            background: "#0f172a".into(),
            text: "#ffffff".into(),
            accent: "#38bdf8".into(),
        }
    }
}

impl ChannelConfig {
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
        let bad = VideoConfig { top_n: 0, top_n_short: 50, seconds_per_year: -1.0, short_seconds: 3.0, ..Default::default() };
        let p = bad.problems().join("\n");
        for key in ["top_n ", "top_n_short", "seconds_per_year", "short_seconds"] {
            assert!(p.contains(key), "missing {key} in {p}");
        }
    }
}
