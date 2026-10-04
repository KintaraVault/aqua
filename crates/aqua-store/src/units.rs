pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().replace('\u{a0}', " ").replace(',', ".");
    if s.is_empty() {
        return None;
    }
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let unit = unit.trim().trim_end_matches([')', '(']);
    let mul: f64 = match unit {
        "" | "B" | "bytes" | "byte" => 1.0,
        "k" | "K" => 1024.0,
        "kB" | "KB" => 1000.0,
        "KiB" | "kiB" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "MB" => 1e6,
        "MiB" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "GB" => 1e9,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TB" => 1e12,
        "TiB" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mul).round() as u64)
}

pub fn format_size(b: u64) -> (String, &'static str) {
    let b = b as f64;
    let (v, u) = if b >= 1e9 {
        (b / 1e9, "GB")
    } else if b >= 1e6 {
        (b / 1e6, "MB")
    } else if b >= 1e3 {
        (b / 1e3, "KB")
    } else {
        return (format!("{}", b as u64), "bytes");
    };
    let s = if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    (s, u)
}

pub fn size_label(b: u64) -> String {
    let (v, u) = format_size(b);
    format!("{v} {u}")
}

pub fn compact_count(n: u64) -> String {
    let f = n as f64;
    if f >= 1e6 {
        trim_dec(f / 1e6, "M")
    } else if f >= 1e3 {
        trim_dec(f / 1e3, "K")
    } else {
        n.to_string()
    }
}

fn trim_dec(v: f64, suffix: &str) -> String {
    let s = if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    let s = s.strip_suffix(".0").map(str::to_string).unwrap_or(s);
    format!("{s}{suffix}")
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    let d = s.get(..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let dd: u32 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&dd) {
        return None;
    }
    Some(days_from_civil(y, m, dd) * 86400)
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn today() -> String {
    let (y, m, d) = civil_from_days(now().div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}

pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub fn short_date(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86400));
    format!("{} {d}, {y}", MONTHS[(m - 1) as usize])
}

pub fn relative_age(ts: i64, now: i64) -> String {
    let d = (now - ts).max(0) / 86400;
    if d < 1 {
        "Today".into()
    } else if d < 2 {
        "Yesterday".into()
    } else if d < 7 {
        format!("{d}d ago")
    } else if d < 31 {
        format!("{}w ago", d / 7)
    } else if d < 365 {
        format!("{}mo ago", d / 30)
    } else {
        format!("{}y ago", d / 365)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("12.5 MiB"), Some(13_107_200));
        assert_eq!(parse_size("1.2 GB"), Some(1_200_000_000));
        assert_eq!(parse_size("640 kB"), Some(640_000));
        assert_eq!(parse_size("25 M"), Some(26_214_400));
        assert_eq!(parse_size("123"), Some(123));
        assert_eq!(parse_size("1,5 MB"), Some(1_500_000));
        assert_eq!(parse_size("huge"), None);
        assert_eq!(size_label(767_300_000), "767 MB");
        assert_eq!(size_label(12_340_000), "12.3 MB");
        assert_eq!(size_label(900), "900 bytes");
    }

    #[test]
    fn counts() {
        assert_eq!(compact_count(85_000), "85K");
        assert_eq!(compact_count(3_863_551), "3.9M");
        assert_eq!(compact_count(999), "999");
    }

    #[test]
    fn dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 3)), (2026, 10, 3));
        assert_eq!(parse_date("2026-10-03"), Some(days_from_civil(2026, 10, 3) * 86400));
        assert_eq!(parse_date("1788998400"), Some(1788998400));
        assert_eq!(short_date(0), "Jan 1, 1970");
        assert_eq!(relative_age(0, 86400 * 40), "1mo ago");
        assert_eq!(relative_age(0, 3600), "Today");
    }
}
