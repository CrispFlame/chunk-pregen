//! Small text helpers for progress messages.

/// 1234567 -> "1,234,567"
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 3725 -> "1h 02m 05s", 125 -> "2m 05s", 9 -> "9s"
pub fn duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn percent(done: u64, total: u64) -> String {
    if total == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", done as f64 * 100.0 / total as f64)
}

/// Time left, given how many chunks remain and the current chunks-per-second.
pub fn eta(remaining: u64, cps: f64) -> String {
    if cps <= 0.0 {
        return "calculating...".to_string();
    }
    duration((remaining as f64 / cps).ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_works() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn duration_works() {
        assert_eq!(duration(9), "9s");
        assert_eq!(duration(125), "2m 05s");
        assert_eq!(duration(3725), "1h 02m 05s");
    }

    #[test]
    fn percent_and_eta() {
        assert_eq!(percent(0, 0), "0.0%");
        assert_eq!(percent(1, 3), "33.3%");
        assert_eq!(eta(100, 0.0), "calculating...");
        assert_eq!(eta(100, 10.0), "10s");
        assert_eq!(eta(101, 10.0), "11s");
    }
}
