use crate::config::{Style, VideoConfig};
use crate::data::{parse, Dataset};
use anyhow::Result;
use std::path::Path;

#[derive(Debug)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub items: usize,
    pub first: f64,
    pub last: f64,
    pub granularity: crate::data::Granularity,
    pub estimated_share: f64,
    pub reviewed: bool,
    pub style: Style,
}

pub fn run(week_dir: &Path) -> Result<Report> {
    // ponytail: load() also range-checks, so bad ranges surface here as one video.toml error
    let (cfg, cfg_err) = match VideoConfig::load(&week_dir.join("video.toml")) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(format!("video.toml: {e:#}"))),
    };
    // a broken video.toml can't say its style: guess from which data file exists
    let pillars_files = week_dir.join("pillars.csv").exists() && !week_dir.join("data.csv").exists();
    match &cfg {
        Some(c) if c.style == Style::Pillars => return run_pillars(week_dir, c.clone(), vec![]),
        None if pillars_files => return run_pillars(week_dir, VideoConfig::default(), cfg_err.into_iter().collect()),
        _ => {}
    }
    let reviewed = cfg.as_ref().is_some_and(|c| c.reviewed);
    let (items, mut errors, g) = parse(week_dir)?;
    errors.extend(cfg_err);
    let mut warnings = Vec::new();
    match crate::events::parse(week_dir) {
        Ok((_, ev_errors)) => errors.extend(ev_errors),
        Err(e) => errors.push(format!("events.csv: {e:#}")),
    }
    if !reviewed {
        warnings.push("reviewed = false: check data.csv, then set reviewed = true in video.toml".into());
    }
    for it in &items {
        if it.points.len() == 1 {
            warnings.push(format!("'{}' has only one data point", it.name));
        }
        if it.sources.is_empty() && it.points.iter().any(|p| !p.estimated) {
            warnings.push(format!("'{}' has unsourced values not marked estimated", it.name));
        }
    }
    let mut ds = Dataset::from_items(items);
    ds.granularity = ds.granularity.max(g);
    Ok(Report {
        errors,
        warnings,
        items: ds.items.len(),
        first: ds.first,
        last: ds.last,
        granularity: ds.granularity,
        estimated_share: ds.estimated_share(),
        reviewed,
        style: Style::Race,
    })
}

fn run_pillars(week_dir: &Path, cfg: VideoConfig, mut errors: Vec<String>) -> Result<Report> {
    let (rows, row_errors, mut warnings) = crate::pillars::parse_with_warnings(week_dir)?;
    errors.extend(row_errors);
    if !cfg.reviewed {
        warnings.push("reviewed = false: check pillars.csv, then set reviewed = true in video.toml".into());
    }
    if cfg.top_n != VideoConfig::default().top_n {
        warnings.push("top_n is ignored for style = \"pillars\"".into());
    }
    if !cfg.pace.is_empty() {
        warnings.push("[[pace]] is ignored for style = \"pillars\" (use seconds_per_pillar)".into());
    }
    let est = rows.iter().filter(|r| r.estimated).count();
    Ok(Report {
        errors,
        warnings,
        items: rows.len(),
        first: 0.0,
        last: 0.0,
        granularity: crate::data::Granularity::Year,
        estimated_share: if rows.is_empty() { 0.0 } else { est as f64 / rows.len() as f64 },
        reviewed: cfg.reviewed,
        style: Style::Pillars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_week, HEADER};
    use std::path::Path;

    #[test]
    fn duplicate_name_year_is_error() {
        let dir = temp_week(&format!("{HEADER}A,X,,2000,1,,false\nA,X,,2000,2,,false\n"));
        let r = run(&dir).unwrap();
        assert!(r.errors.iter().any(|e| e.contains("duplicate")), "{:?}", r.errors);
    }

    #[test]
    fn missing_image_is_error() {
        let dir = temp_week(&format!("{HEADER}A,X,images/nope.png,2000,1,,false\n"));
        let r = run(&dir).unwrap();
        assert!(r.errors.iter().any(|e| e.contains("nope.png")), "{:?}", r.errors);
    }

    #[test]
    fn bad_number_is_error_with_line() {
        let dir = temp_week(&format!("{HEADER}A,X,,2000,1,,false\nA,X,,2001,abc,,false\n"));
        let r = run(&dir).unwrap();
        assert!(r.errors.iter().any(|e| e.contains("line 3")), "{:?}", r.errors);
    }

    #[test]
    fn report_summary() {
        let r = run(Path::new("tests/fixtures/week")).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(r.reviewed);
        assert_eq!(r.items, 4);
        assert!(r.estimated_share > 0.0 && r.estimated_share < 1.0);
    }

    #[test]
    fn validate_pillars_week() {
        let dir = crate::testutil::temp_week("name\n");
        std::fs::write(dir.join("pillars.csv"), "label,name,value\n1,A,5\n2,B,7\n").unwrap();
        std::fs::write(dir.join("video.toml"), "title = \"T\"\nstyle = \"pillars\"\ntop_n = 12\nreviewed = true\n").unwrap();
        let r = run(&dir).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.items, 2);
        assert!(r.warnings.iter().any(|w| w.contains("top_n")), "{:?}", r.warnings);
    }

    #[test]
    fn pillars_config_error_not_masked() {
        let dir = crate::testutil::temp_week("name\n");
        std::fs::remove_file(dir.join("data.csv")).unwrap();
        std::fs::write(dir.join("pillars.csv"), "label,name,value\n1,A,5\n").unwrap();
        std::fs::write(dir.join("video.toml"), "style = \"pillars\"\nseconds_per_pillar = 0\n").unwrap();
        let r = run(&dir).unwrap();
        assert!(r.errors.iter().any(|e| e.contains("seconds_per_pillar")), "{:?}", r.errors);
    }
}
