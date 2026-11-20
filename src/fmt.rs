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
}
