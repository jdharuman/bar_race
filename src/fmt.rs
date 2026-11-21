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

/// Human-readable number: one decimal below 100, otherwise rounded with thousands separators.
pub fn format_value(v: f64, prefix: &str, suffix: &str) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let num = if a < 100.0 {
        format!("{a:.1}")
    } else {
        let digits = format!("{:.0}", a.round());
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    };
    format!("{sign}{prefix}{num}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ranges() {
        assert_eq!(format_value(0.5, "$", "B"), "$0.5B");
        assert_eq!(format_value(99.94, "", ""), "99.9");
        assert_eq!(format_value(1234.5, "", ""), "1,235");
        assert_eq!(format_value(1234567.0, "$", ""), "$1,234,567");
        assert_eq!(format_value(2.4e12, "", ""), "2,400,000,000,000");
        assert_eq!(format_value(-5.0, "", ""), "-5.0");
        assert_eq!(format_value(-1500.0, "$", ""), "-$1,500");
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
