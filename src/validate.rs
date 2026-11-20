use crate::config::VideoConfig;
use crate::data::{parse, Dataset};
use anyhow::Result;
use std::path::Path;

#[derive(Debug)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub items: usize,
    pub first_year: i32,
    pub last_year: i32,
    pub estimated_share: f64,
    pub reviewed: bool,
}

pub fn run(week_dir: &Path) -> Result<Report> {
    let (items, mut errors) = parse(week_dir)?;
    let mut warnings = Vec::new();
    // ponytail: load() also range-checks, so bad ranges surface here as one video.toml error
    let reviewed = match VideoConfig::load(&week_dir.join("video.toml")) {
        Ok(c) => c.reviewed,
        Err(e) => {
            errors.push(format!("video.toml: {e:#}"));
            false
        }
    };
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
    let ds = Dataset::from_items(items);
    Ok(Report {
        errors,
        warnings,
        items: ds.items.len(),
        first_year: ds.first_year,
        last_year: ds.last_year,
        estimated_share: ds.estimated_share(),
        reviewed,
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
}
