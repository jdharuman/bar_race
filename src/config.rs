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
    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
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
