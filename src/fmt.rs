use crate::data::Granularity;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Counter label for fractional-year time `t`: "1999", "Feb 1999" or "Q2 1999".
pub fn format_time(t: f64, g: Granularity) -> String {
    let t = t + 1e-6; // float drift (1999.0833333 - ε) still lands in the right month
    let year = t.floor();
    let frac = t - year;
    match g {
        Granularity::Year => format!("{year:.0}"),
        Granularity::Quarter => format!("Q{} {year:.0}", ((frac * 4.0).floor() as usize).min(3) + 1),
        Granularity::Month => format!("{} {year:.0}", MONTHS[((frac * 12.0).floor() as usize).min(11)]),
    }
}

/// How value labels and axis ticks are written.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberStyle {
    /// Every digit: 77,246,849,166 (low digits spin as values change).
    #[default]
    Full,
    /// Short form: 77.2B.
    Compact,
}

fn with_commas(a: f64) -> String {
    let digits = format!("{:.0}", a.round());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn format_value(v: f64, prefix: &str, suffix: &str, style: NumberStyle) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let num = match style {
        NumberStyle::Full if a < 1000.0 => format!("{a:.1}"),
        NumberStyle::Full => with_commas(a),
        NumberStyle::Compact => {
            const UNITS: [(f64, &str); 4] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")];
            match UNITS.iter().find(|(u, _)| a >= *u) {
                Some((u, s)) => format!("{:.1}{s}", a / u),
                None => format!("{a:.0}"),
            }
        }
    };
    format!("{sign}{prefix}{num}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_full_and_compact() {
        use NumberStyle::*;
        assert_eq!(format_value(77_246_849_166.4, "$", "", Full), "$77,246,849,166");
        assert_eq!(format_value(999.94, "", "", Full), "999.9");
        assert_eq!(format_value(1000.4, "", "", Full), "1,000");
        assert_eq!(format_value(-1500.0, "$", "", Full), "-$1,500");
        assert_eq!(format_value(0.5, "", "", Full), "0.5");
        assert_eq!(format_value(99.96, "", "", Full), "100.0");
        assert_eq!(format_value(950.0, "", "", Compact), "950");
        assert_eq!(format_value(1_234.0, "", "", Compact), "1.2K");
        assert_eq!(format_value(3_400_000.0, "$", "", Compact), "$3.4M");
        assert_eq!(format_value(2.4e12, "", "", Compact), "2.4T");
        assert_eq!(format_value(7.5e9, "", "", Compact), "7.5B");
    }

    #[test]
    fn number_style_from_toml() {
        let c: crate::config::VideoConfig = toml::from_str("number_style = \"compact\"").unwrap();
        assert_eq!(c.number_style, NumberStyle::Compact);
        assert_eq!(crate::config::VideoConfig::default().number_style, NumberStyle::Full);
    }

    #[test]
    fn formats_time_labels() {
        use crate::data::Granularity;
        assert_eq!(format_time(1999.0, Granularity::Year), "1999");
        assert_eq!(format_time(1999.0 + 1.0 / 12.0 + 0.01, Granularity::Month), "Feb 1999");
        assert_eq!(format_time(1999.0 + 1.0 / 12.0 - 1e-9, Granularity::Month), "Feb 1999");
        assert_eq!(format_time(1999.26, Granularity::Quarter), "Q2 1999");
        assert_eq!(format_time(1999.999, Granularity::Year), "1999");
        assert_eq!(format_time(1999.9999999999, Granularity::Year), "2000");
    }
}
