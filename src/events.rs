use crate::data::parse_time;
use anyhow::{Context, Result};
use std::path::Path;

/// A fact caption shown while data time is in [from, to).
#[derive(Debug, Clone)]
pub struct Event {
    pub from: f64,
    pub to: f64,
    pub headline: String,
    pub big: String,
    pub caption: String,
}

/// Read `events.csv` from the week folder. A missing file means no events.
pub fn parse(week_dir: &Path) -> Result<(Vec<Event>, Vec<String>)> {
    let path = week_dir.join("events.csv");
    if !path.exists() {
        return Ok((vec![], vec![]));
    }
    let mut rdr = csv::Reader::from_path(&path).with_context(|| format!("reading {}", path.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h.trim() == n);
    let cols: Vec<Option<usize>> = ["from", "to", "headline", "big", "caption"].iter().map(|n| col(n)).collect();
    if cols.iter().any(|c| c.is_none()) {
        return Ok((vec![], vec!["events.csv: needs columns from,to,headline,big,caption".into()]));
    }
    let (mut events, mut errors) = (Vec::new(), Vec::new());
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 2;
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("events.csv line {line}: {e}"));
                continue;
            }
        };
        let get = |k: usize| rec.get(cols[k].unwrap()).unwrap_or("").trim().to_string();
        let (Some((from, _)), Some((to, _))) = (parse_time(&get(0)), parse_time(&get(1))) else {
            errors.push(format!("events.csv line {line}: bad from/to (use 1999, 1999-02 or 1999-Q2)"));
            continue;
        };
        if from >= to {
            errors.push(format!("events.csv line {line}: from must be before to"));
            continue;
        }
        events.push(Event { from, to, headline: get(2), big: get(3), caption: get(4) });
    }
    Ok((events, errors))
}

/// Index of the event showing at time `t`: among those covering t, the latest `from`.
pub fn active(events: &[Event], t: f64) -> Option<usize> {
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from <= t && t < e.to)
        .max_by(|a, b| a.1.from.total_cmp(&b.1.from))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_week;

    fn week_with_events(csv: &str) -> std::path::PathBuf {
        let dir = temp_week("name,category,image,year,value,source,estimated\n");
        std::fs::write(dir.join("events.csv"), csv).unwrap();
        dir
    }

    #[test]
    fn parses_events() {
        let dir = week_with_events("from,to,headline,big,caption\n1913,1925,US produces,90%,World's Oil\n");
        let (ev, errs) = parse(&dir).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].from, 1913.0);
        assert_eq!(ev[0].big, "90%");
    }

    #[test]
    fn bad_rows_reported_with_line() {
        let dir = week_with_events("from,to,headline,big,caption\n1925,1913,a,b,c\nxx,1930,a,b,c\n");
        let (ev, errs) = parse(&dir).unwrap();
        assert!(ev.is_empty());
        assert!(errs.iter().any(|e| e.contains("line 2")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("line 3")), "{errs:?}");
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = temp_week("name\n");
        let (ev, errs) = parse(&dir).unwrap();
        assert!(ev.is_empty() && errs.is_empty());
    }

    #[test]
    fn later_from_wins() {
        let mk = |from: f64, to: f64| Event { from, to, headline: String::new(), big: String::new(), caption: String::new() };
        let ev = vec![mk(1900.0, 1950.0), mk(1920.0, 1930.0)];
        assert_eq!(active(&ev, 1925.0), Some(1));
        assert_eq!(active(&ev, 1935.0), Some(0));
        assert_eq!(active(&ev, 1950.0), None);
        assert_eq!(active(&ev, 1899.0), None);
    }
}
