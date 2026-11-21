use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub year: i32,
    pub value: f64,
    pub estimated: bool,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub category: String,
    pub image: Option<PathBuf>,
    pub points: Vec<Point>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Dataset {
    pub items: Vec<Item>,
    pub first_year: i32,
    pub last_year: i32,
}

/// Parse data.csv leniently: returns whatever items parsed plus every problem found.
pub fn parse(week_dir: &Path) -> Result<(Vec<Item>, Vec<String>)> {
    let path = week_dir.join("data.csv");
    let mut rdr = csv::Reader::from_path(&path).with_context(|| format!("reading {}", path.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h.trim() == n);
    let mut errors = Vec::new();
    let mut idx = BTreeMap::new();
    for n in ["name", "category", "image", "year", "value", "source", "estimated"] {
        match col(n) {
            Some(i) => {
                idx.insert(n, i);
            }
            None => errors.push(format!("missing column '{n}'")),
        }
    }
    if !errors.is_empty() {
        return Ok((vec![], errors));
    }
    let mut items: Vec<Item> = Vec::new();
    let mut seen = HashSet::new();
    let mut missing_images = HashSet::new();
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 2;
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("line {line}: {e}"));
                continue;
            }
        };
        let get = |n: &str| rec.get(idx[n]).unwrap_or("").trim();
        let name = get("name").to_string();
        if name.is_empty() {
            errors.push(format!("line {line}: empty name"));
            continue;
        }
        let Ok(year) = get("year").parse::<i32>() else {
            errors.push(format!("line {line}: bad year '{}'", get("year")));
            continue;
        };
        let Ok(value) = get("value").parse::<f64>() else {
            errors.push(format!("line {line}: bad value '{}'", get("value")));
            continue;
        };
        if !value.is_finite() {
            errors.push(format!("line {line}: value not finite"));
            continue;
        }
        let estimated = match get("estimated").to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" | "" => false,
            other => {
                errors.push(format!("line {line}: bad estimated '{other}'"));
                continue;
            }
        };
        if !seen.insert((name.clone(), year)) {
            errors.push(format!("line {line}: duplicate row for '{name}' {year}"));
            continue;
        }
        let image = match get("image") {
            "" => None,
            p => {
                let full = week_dir.join(p);
                if !full.exists() && missing_images.insert(p.to_string()) {
                    errors.push(format!("line {line}: image not found: {p}"));
                }
                Some(full)
            }
        };
        let source = get("source").to_string();
        let item = match items.iter_mut().find(|it| it.name == name) {
            Some(it) => it,
            None => {
                items.push(Item {
                    name: name.clone(),
                    category: get("category").to_string(),
                    image: image.clone(),
                    points: vec![],
                    sources: vec![],
                });
                items.last_mut().unwrap()
            }
        };
        if item.image.is_none() {
            item.image = image;
        }
        if !source.is_empty() && !item.sources.contains(&source) {
            item.sources.push(source);
        }
        item.points.push(Point { year, value, estimated });
    }
    for it in &mut items {
        it.points.sort_by_key(|p| p.year);
    }
    if items.is_empty() && errors.is_empty() {
        errors.push("no data rows".into());
    }
    Ok((items, errors))
}

impl Dataset {
    pub fn from_items(items: Vec<Item>) -> Dataset {
        let years = items.iter().flat_map(|i| i.points.iter().map(|p| p.year));
        let first_year = years.clone().min().unwrap_or(0);
        let last_year = years.max().unwrap_or(0);
        Dataset { items, first_year, last_year }
    }

    pub fn load(week_dir: &Path) -> Result<Dataset> {
        let (items, errors) = parse(week_dir)?;
        if !errors.is_empty() {
            bail!("data.csv has errors:\n  {}", errors.join("\n  "));
        }
        Ok(Dataset::from_items(items))
    }

    pub fn estimated_share(&self) -> f64 {
        let (mut est, mut all) = (0usize, 0usize);
        for p in self.items.iter().flat_map(|i| &i.points) {
            all += 1;
            est += p.estimated as usize;
        }
        if all == 0 { 0.0 } else { est as f64 / all as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_week, HEADER};
    use std::path::Path;

    #[test]
    fn loads_fixture() {
        let d = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        assert_eq!(d.items.len(), 4);
        assert_eq!((d.first_year, d.last_year), (2000, 2003));
        assert!(d.items.iter().all(|i| i.points.windows(2).all(|w| w[0].year < w[1].year)));
    }

    #[test]
    fn quoted_names_with_commas() {
        let dir = temp_week(&format!("{HEADER}\"Korea, South\",Asia,,2000,1,,false\n"));
        let d = Dataset::load(&dir).unwrap();
        assert_eq!(d.items[0].name, "Korea, South");
    }

    #[test]
    fn estimated_share_counts_points() {
        let d = Dataset::load(Path::new("tests/fixtures/week")).unwrap();
        assert!((d.estimated_share() - 1.0 / 13.0).abs() < 1e-9);
    }
}
