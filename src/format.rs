//! Human-readable formatting shared by the terminal and web output.

use chrono::TimeDelta;

/// Formats a byte count with binary units, e.g. `1.5 KB`.
pub fn bytes(n: u64) -> String {
    const UNIT: u64 = 1024;
    if n < UNIT {
        return format!("{n} B");
    }
    let (mut div, mut exp) = (UNIT, 0);
    let mut rest = n / UNIT;
    while rest >= UNIT {
        div *= UNIT;
        exp += 1;
        rest /= UNIT;
    }
    let prefix = b"KMGTPE"[exp] as char;
    format!("{:.1} {prefix}B", n as f64 / div as f64)
}

/// Formats an hour of the day on a 12-hour clock, e.g. `3:00 PM`.
pub fn hour(h: u32) -> String {
    match h {
        0 => "12:00 AM".to_string(),
        1..=11 => format!("{h}:00 AM"),
        12 => "12:00 PM".to_string(),
        _ => format!("{}:00 PM", h - 12),
    }
}

/// Formats a count with a unit that takes a plain `s` in the plural,
/// e.g. `1 day` or `3 days`.
pub fn units(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// Formats a duration as days, or hours or minutes when shorter than a day.
/// `short` writes minutes as `min`.
pub fn duration(d: TimeDelta, short: bool) -> String {
    match d.num_days() {
        0 if d.num_hours() == 0 && short => format!("{} min", d.num_minutes()),
        0 if d.num_hours() == 0 => units(d.num_minutes(), "minute"),
        0 => units(d.num_hours(), "hour"),
        days => units(days, "day"),
    }
}

/// Formats an integer with thousands separators, e.g. `12,345`.
pub fn count(n: impl Into<i128>) -> String {
    let n: i128 = n.into();
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shortens a string to `max` characters, ending with `...` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return s.chars().take(max).collect();
    }
    let mut out: String = s.chars().take(max - 3).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn formats_hours() {
        assert_eq!(hour(0), "12:00 AM");
        assert_eq!(hour(9), "9:00 AM");
        assert_eq!(hour(12), "12:00 PM");
        assert_eq!(hour(23), "11:00 PM");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(TimeDelta::minutes(42), true), "42 min");
        assert_eq!(duration(TimeDelta::minutes(1), false), "1 minute");
        assert_eq!(duration(TimeDelta::minutes(42), false), "42 minutes");
        assert_eq!(duration(TimeDelta::hours(1), false), "1 hour");
        assert_eq!(duration(TimeDelta::hours(5), false), "5 hours");
        assert_eq!(duration(TimeDelta::hours(30), false), "1 day");
        assert_eq!(duration(TimeDelta::days(12), true), "12 days");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1234567u64), "1,234,567");
        assert_eq!(count(-45678), "-45,678");
    }

    #[test]
    fn truncates_by_chars() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("привет", 3), "при");
    }
}
