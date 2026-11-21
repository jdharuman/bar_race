use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// How finely the data is sampled. Ordered coarse → fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Granularity {
    Year,
    Quarter,
    Month,
}

/// "1999" → 1999.0, "1999-02" → 1999 + 1/12, "1999-Q2" → 1999.25 (fractional years).
pub fn parse_time(s: &str) -> Option<(f64, Granularity)> {
    let s = s.trim();
    // optional minus sign (BCE) then 1–4 year digits
    let sign_len = usize::from(s.starts_with('-'));
    let digits = s[sign_len..].bytes().take_while(|b| b.is_ascii_digit()).count();
    if !(1..=4).contains(&digits) {
        return None;
    }
    let year: f64 = s[..sign_len + digits].parse().ok()?;
    let rest = &s[sign_len + digits..];
    if rest.is_empty() {
        return Some((year, Granularity::Year));
    }
    let rest = rest.strip_prefix('-')?;
    if let Some(q) = rest.strip_prefix('Q').or_else(|| rest.strip_prefix('q')) {
        let q: u32 = q.parse().ok().filter(|q| (1..=4).contains(q))?;
        return Some((year + (q - 1) as f64 / 4.0, Granularity::Quarter));
    }
    if rest.len() == 2 && rest.bytes().all(|b| b.is_ascii_digit()) {
        let m: u32 = rest.parse().ok().filter(|m| (1..=12).contains(m))?;
        return Some((year + (m - 1) as f64 / 12.0, Granularity::Month));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Time in fractional years.
    pub t: f64,
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
    pub first: f64,
    pub last: f64,
    pub granularity: Granularity,
}

/// Parse data.csv leniently: returns whatever items parsed plus every problem found.
/// Also returns the finest granularity written in the `year` column.
pub fn parse(week_dir: &Path) -> Result<(Vec<Item>, Vec<String>, Granularity)> {
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
        return Ok((vec![], errors, Granularity::Year));
    }
    let mut items: Vec<Item> = Vec::new();
    let mut seen = HashSet::new();
    let mut missing_images = HashSet::new();
    let mut granularity = Granularity::Year;
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
        let Some((t, g)) = parse_time(get("year")) else {
            errors.push(format!("line {line}: bad year '{}' (use 1999, 1999-02 or 1999-Q2)", get("year")));
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
        if !seen.insert((name.clone(), (t * 1e6).round() as i64)) {
            errors.push(format!("line {line}: duplicate row for '{name}' {}", get("year")));
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
        item.points.push(Point { t, value, estimated });
        granularity = granularity.max(g);
    }
    for it in &mut items {
        it.points.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
    if items.is_empty() && errors.is_empty() {
        errors.push("no data rows".into());
    }
    Ok((items, errors, granularity))
}

/// Finest granularity among all points: a point off whole years/quarters means finer data.
fn granularity_of(items: &[Item]) -> Granularity {
    let mut g = Granularity::Year;
    for p in items.iter().flat_map(|i| &i.points) {
        let frac = p.t - p.t.floor();
        let is = |n: f64| ((frac * n) - (frac * n).round()).abs() < 1e-6;
        if !is(1.0) {
            g = g.max(if is(4.0) { Granularity::Quarter } else { Granularity::Month });
        }
    }
    g
}

impl Dataset {
    pub fn from_items(items: Vec<Item>) -> Dataset {
        let ts = items.iter().flat_map(|i| i.points.iter().map(|p| p.t));
        let first = ts.clone().fold(f64::INFINITY, f64::min);
        let last = ts.fold(f64::NEG_INFINITY, f64::max);
        let (first, last) = if first.is_finite() { (first, last) } else { (0.0, 0.0) };
        let granularity = granularity_of(&items);
        Dataset { items, first, last, granularity }
    }

    /// Length of one data step in years.
    pub fn step(&self) -> f64 {
        match self.granularity {
            Granularity::Year => 1.0,
            Granularity::Quarter => 0.25,
            Granularity::Month => 1.0 / 12.0,
        }
    }

    pub fn load(week_dir: &Path) -> Result<Dataset> {
        let (items, errors, g) = parse(week_dir)?;
        if !errors.is_empty() {
            bail!("data.csv has errors:\n  {}", errors.join("\n  "));
        }
        let mut ds = Dataset::from_items(items);
        ds.granularity = ds.granularity.max(g);
        Ok(ds)
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
        assert_eq!((d.first, d.last), (2000.0, 2003.0));
        assert!(d.items.iter().all(|i| i.points.windows(2).all(|w| w[0].t < w[1].t)));
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

    #[test]
    fn parses_time_formats() {
        assert_eq!(parse_time("1999"), Some((1999.0, Granularity::Year)));
        assert_eq!(parse_time("1999-02"), Some((1999.0 + 1.0 / 12.0, Granularity::Month)));
        assert_eq!(parse_time("1999-Q2"), Some((1999.25, Granularity::Quarter)));
        assert_eq!(parse_time("1999-q4"), Some((1999.75, Granularity::Quarter)));
        assert_eq!(parse_time("1999-13"), None);
        assert_eq!(parse_time("1999-00"), None);
        assert_eq!(parse_time("1999-Q5"), None);
        assert_eq!(parse_time("abc"), None);
    }

    #[test]
    fn mixed_granularity_uses_finest() {
        let dir = temp_week(&format!("{HEADER}A,X,,2000-06,2,,false\nA,X,,2000,1,,false\n"));
        let d = Dataset::load(&dir).unwrap();
        assert_eq!(d.granularity, Granularity::Month);
        assert!((d.step() - 1.0 / 12.0).abs() < 1e-12);
        assert_eq!(d.items[0].points[0].t, 2000.0);
        assert!((d.items[0].points[1].t - 2000.4166666).abs() < 1e-6);
    }

    #[test]
    fn duplicate_same_time_different_spelling() {
        let dir = temp_week(&format!("{HEADER}A,X,,2000,1,,false\nA,X,,2000-01,2,,false\n"));
        let (_, errs, _) = parse(&dir).unwrap();
        assert!(errs.iter().any(|e| e.contains("duplicate")), "{errs:?}");
    }

    #[test]
    fn short_and_negative_years() {
        assert_eq!(parse_time("800"), Some((800.0, Granularity::Year)));
        assert_eq!(parse_time("-500"), Some((-500.0, Granularity::Year)));
        assert_eq!(parse_time("45-03"), Some((45.0 + 2.0 / 12.0, Granularity::Month)));
        assert_eq!(parse_time("12345"), None);
        assert_eq!(parse_time("-"), None);
    }
}
