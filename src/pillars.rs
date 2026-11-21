use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// One pillar of the walk: a row of pillars.csv.
#[derive(Debug, Clone)]
pub struct Pillar {
    pub label: String,
    pub name: String,
    pub value: f64,
    pub detail: String,
    pub image: Option<PathBuf>,
    pub source: String,
    pub estimated: bool,
}

/// Read pillars.csv leniently: the rows that parsed plus every problem (with line numbers).
pub fn parse(week_dir: &Path) -> Result<(Vec<Pillar>, Vec<String>)> {
    let path = week_dir.join("pillars.csv");
    let mut rdr = csv::Reader::from_path(&path).with_context(|| format!("reading {}", path.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h.trim() == n);
    let mut errors = Vec::new();
    for req in ["label", "name", "value"] {
        if col(req).is_none() {
            errors.push(format!("pillars.csv: missing column '{req}'"));
        }
    }
    if !errors.is_empty() {
        return Ok((vec![], errors));
    }
    let cols: Vec<Option<usize>> = ["label", "name", "value", "detail", "image", "source", "estimated"].iter().map(|n| col(n)).collect();
    let mut rows = Vec::new();
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 2;
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("pillars.csv line {line}: {e}"));
                continue;
            }
        };
        let get = |k: usize| cols[k].and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        let name = get(1);
        if name.is_empty() {
            errors.push(format!("pillars.csv line {line}: empty name"));
            continue;
        }
        let value = match get(2).parse::<f64>() {
            Ok(v) if v.is_finite() && v >= 0.0 => v,
            _ => {
                errors.push(format!("pillars.csv line {line}: value must be a number ≥ 0 (got '{}')", get(2)));
                continue;
            }
        };
        let estimated = match get(6).to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" | "" => false,
            other => {
                errors.push(format!("pillars.csv line {line}: bad estimated '{other}'"));
                continue;
            }
        };
        let image = match get(4) {
            p if p.is_empty() => None,
            p => {
                let full = week_dir.join(&p);
                if !full.exists() {
                    errors.push(format!("pillars.csv line {line}: image not found: {p}"));
                }
                Some(full)
            }
        };
        rows.push(Pillar { label: get(0), name, value, detail: get(3), image, source: get(5), estimated });
    }
    if rows.is_empty() && errors.is_empty() {
        errors.push("pillars.csv: no rows".into());
    }
    Ok((rows, errors))
}

pub fn load(week_dir: &Path) -> Result<Vec<Pillar>> {
    let (rows, errors) = parse(week_dir)?;
    if !errors.is_empty() {
        bail!("pillars.csv has errors:\n  {}", errors.join("\n  "));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_week;

    fn week(csv: &str) -> std::path::PathBuf {
        let dir = temp_week("name\n");
        std::fs::write(dir.join("pillars.csv"), csv).unwrap();
        dir
    }

    #[test]
    fn parses_rows_in_order() {
        let dir = week("label,name,value,detail,image,source,estimated\n1946-47,Joe Fulks,8000,Philadelphia Warriors,,https://x.org/a,false\n1947-48,George Mikan,12500,Minneapolis Lakers,,,true\n1945-46,Zed,1,,,,\n");
        let (rows, errs) = parse(&dir).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Joe Fulks", "George Mikan", "Zed"]);
        assert_eq!(rows[0].value, 8000.0);
        assert_eq!(rows[0].detail, "Philadelphia Warriors");
        assert!(rows[1].estimated);
    }

    #[test]
    fn optional_columns_may_be_missing() {
        let dir = week("label,name,value\n2001,A,5\n");
        let (rows, errs) = parse(&dir).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(rows[0].image, None);
        assert!(!rows[0].estimated);
    }

    #[test]
    fn bad_rows_have_line_numbers() {
        let dir = week("label,name,value\n1,A,-1\n2,B,abc\n3,,5\n");
        let (_, errs) = parse(&dir).unwrap();
        for n in ["line 2", "line 3", "line 4"] {
            assert!(errs.iter().any(|e| e.contains(n)), "{n}: {errs:?}");
        }
    }

    #[test]
    fn missing_image_is_error() {
        let dir = week("label,name,value,image\n1,A,5,images/nope.jpg\n");
        let (_, errs) = parse(&dir).unwrap();
        assert!(errs.iter().any(|e| e.contains("nope.jpg")), "{errs:?}");
    }

    #[test]
    fn missing_required_column() {
        let dir = week("label,value\n1,5\n");
        let (_, errs) = parse(&dir).unwrap();
        assert!(errs.iter().any(|e| e.contains("name")), "{errs:?}");
    }
}
