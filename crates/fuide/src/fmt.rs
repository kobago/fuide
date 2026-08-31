//! Telemetry-style number / time formatting.

/// Mission-clock uptime: `T+00:12:34.5`; past 24 h a day count is prefixed: `T+1d 03:22:11.4`.
/// Width is fixed within a day, so the status bar does not jitter. `f64` seconds never overflow
/// in practice (0.1 s resolution is kept up to ~2^53 s).
pub fn uptime(secs: f64) -> String {
    let secs = secs.max(0.0);
    let total_tenths = (secs * 10.0).floor() as u64;
    let tenths = total_tenths % 10;
    let s = total_tenths / 10;
    let (d, h, m, sec) = (s / 86_400, (s / 3_600) % 24, (s / 60) % 60, s % 60);
    if d > 0 {
        format!("T+{d}d {h:02}:{m:02}:{sec:02}.{tenths}")
    } else {
        format!("T+{h:02}:{m:02}:{sec:02}.{tenths}")
    }
}

#[cfg(test)]
mod tests {
    use super::uptime;

    #[test]
    fn formats_fixed_width_within_a_day() {
        assert_eq!(uptime(0.0), "T+00:00:00.0");
        assert_eq!(uptime(12.45), "T+00:00:12.4");
        assert_eq!(uptime(3_723.9), "T+01:02:03.9");
        assert_eq!(uptime(86_399.99), "T+23:59:59.9");
    }

    #[test]
    fn prefixes_days_after_24h() {
        assert_eq!(uptime(86_400.0), "T+1d 00:00:00.0");
        assert_eq!(
            uptime(86_400.0 + 3_600.0 * 3.0 + 60.0 * 22.0 + 11.4),
            "T+1d 03:22:11.4"
        );
        assert_eq!(uptime(365.0 * 86_400.0 * 3.0), "T+1095d 00:00:00.0");
    }

    #[test]
    fn never_panics_on_extreme_input() {
        let _ = uptime(-5.0);
        let _ = uptime(f64::MAX);
    }
}
