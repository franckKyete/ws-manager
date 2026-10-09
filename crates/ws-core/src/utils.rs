use chrono::{DateTime, Utc};
use regex::Regex;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

lazy_static::lazy_static! {
    static ref DURATION_REGEX: Regex = Regex::new(r"^(\d+(?:\.\d+)?)\s*([a-zA-Z]+)?$").unwrap();
}

pub fn get_iso_timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn ensure_directory(path: &Path) -> std::io::Result<PathBuf> {
    create_dir_all(path)?;
    Ok(path.to_path_buf())
}

pub fn parse_duration(val: &str) -> u64 {
    let s = val.trim().to_lowercase();
    if s.is_empty()
        || s == "never"
        || s == "none"
        || s == "false"
        || s == "off"
        || s == "0"
        || s == "0s"
        || s == "0m"
        || s == "0h"
    {
        return 0;
    }

    if let Some(caps) = DURATION_REGEX.captures(&s) {
        let amount: f64 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
        let unit = caps.get(2).map(|m| m.as_str()).unwrap_or("s");

        match unit {
            "s" | "sec" | "second" | "seconds" => amount.max(0.0) as u64,
            "m" | "min" | "minute" | "minutes" => (amount * 60.0).max(0.0) as u64,
            "h" | "hr" | "hour" | "hours" => (amount * 3600.0).max(0.0) as u64,
            "d" | "day" | "days" => (amount * 86400.0).max(0.0) as u64,
            "w" | "week" | "weeks" => (amount * 604800.0).max(0.0) as u64,
            _ => amount.max(0.0) as u64,
        }
    } else {
        s.parse::<f64>().map(|n| n.max(0.0) as u64).unwrap_or(0)
    }
}

pub fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_string();
    }
    if seconds % 86400 == 0 {
        return format!("{}d", seconds / 86400);
    }
    if seconds % 3600 == 0 {
        return format!("{}h", seconds / 3600);
    }
    if seconds % 60 == 0 {
        return format!("{}m", seconds / 60);
    }
    format!("{}s", seconds)
}

pub fn format_relative_time(timestamp_str: &str) -> String {
    let parsed: Result<DateTime<Utc>, _> = DateTime::parse_from_rfc3339(timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(timestamp_str, "%Y-%m-%dT%H:%M:%S")
                .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
        });

    if let Ok(dt) = parsed {
        let now = Utc::now();
        let diff = now.signed_duration_since(dt);
        let seconds = diff.num_seconds();

        if seconds < 60 {
            "just now".to_string()
        } else if seconds < 3600 {
            format!("{}m ago", seconds / 60)
        } else if seconds < 86400 {
            format!("{}h ago", seconds / 3600)
        } else if seconds < 172800 {
            "yesterday".to_string()
        } else {
            format!("{} days ago", seconds / 86400)
        }
    } else {
        timestamp_str.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_durations() {
        assert_eq!(parse_duration("15m"), 900);
        assert_eq!(parse_duration("5m"), 300);
        assert_eq!(parse_duration("1h"), 3600);
        assert_eq!(parse_duration("30s"), 30);
        assert_eq!(parse_duration("2d"), 172800);
        assert_eq!(parse_duration("600"), 600);
        assert_eq!(parse_duration("0"), 0);
        assert_eq!(parse_duration("never"), 0);
        assert_eq!(parse_duration("none"), 0);
        assert_eq!(parse_duration("false"), 0);

        assert_eq!(format_duration(300), "5m");
        assert_eq!(format_duration(900), "15m");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(7200), "2h");
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(86400), "1d");
        assert_eq!(format_duration(172800), "2d");
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(65), "65s");
    }
}
