/// Seconds since the Unix epoch, right now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "3m ago", "2h ago", "5d ago", "4w ago", "7mo ago", "2y ago".
pub fn relative_time(then: i64, now: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        86400..604800 => format!("{}d ago", s / 86400),
        604800..2_592_000 => format!("{}w ago", s / 604800),
        // Days 360 to 364 would be "12mo".
        2_592_000..31_536_000 => format!("{}mo ago", (s / 2_592_000).min(11)),
        _ => format!("{}y ago", s / 31_536_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(relative_time(100, 130), "now");
        assert_eq!(relative_time(0, 7200), "2h ago");
        assert_eq!(relative_time(50, 0), "now");
        assert_eq!(relative_time(0, 362 * 86400), "11mo ago");
    }
}
