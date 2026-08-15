//! F7 — the heartbeat line: one log line every ~10 min so a weeks-later field
//! failure is diagnosable from the journal after the fact (was it degrading?
//! hot? dropping frames?). Formatting is pure and unit-tested here;
//! scheduling and the property/sysfs reads live in main.rs.

/// Render one heartbeat line. `temp_millicelsius` comes from
/// /sys/class/thermal (None off-Linux or on read failure); the two counters
/// are mpv property strings (None if the property is unavailable). Missing
/// sources degrade to "n/a" — a heartbeat must never itself be a failure.
pub fn format_heartbeat(
    wraps: u64,
    uptime_secs: u64,
    temp_millicelsius: Option<i64>,
    frame_drops: Option<&str>,
    vo_delayed: Option<&str>,
) -> String {
    let temp = match temp_millicelsius {
        Some(m) => {
            // Extract the sign before dividing: `m / 1000` truncates toward
            // zero, so for m in -999..=-1 it is 0 -- an unheated venue on a
            // winter cold-boot (e.g. -250 m°C) would otherwise render as a
            // plausible-looking POSITIVE "0.2C" in the journal. `unsigned_abs`
            // sidesteps the one panic hazard in this shape (`i64::MIN.abs()`)
            // entirely, though sysfs never reports a value near it.
            let (sign, mag) = if m < 0 { ("-", m.unsigned_abs()) } else { ("", m as u64) };
            format!("{sign}{}.{}C", mag / 1000, (mag % 1000) / 100)
        }
        None => "n/a".to_string(),
    };
    format!(
        "dex-loop: heartbeat wraps={wraps} uptime={uptime_secs}s temp={temp} \
         frame-drops={} vo-delayed={}",
        frame_drops.unwrap_or("n/a"),
        vo_delayed.unwrap_or("n/a"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_all_fields() {
        assert_eq!(
            format_heartbeat(143, 3600, Some(48_250), Some("0"), Some("2")),
            "dex-loop: heartbeat wraps=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=2"
        );
    }

    #[test]
    fn sub_zero_temperatures_keep_their_sign() {
        // -250 m°C: an unheated venue on a winter cold-boot. Before the fix,
        // `m / 1000 == 0` for any m in -999..=-1, so this silently rendered
        // as the POSITIVE "0.2C" -- a plausible-looking wrong reading in
        // exactly the diagnostic line F7 exists to make trustworthy.
        assert_eq!(
            format_heartbeat(0, 0, Some(-250), None, None),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=-0.2C frame-drops=n/a vo-delayed=n/a"
        );
        assert_eq!(
            format_heartbeat(0, 0, Some(-1_500), None, None),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=-1.5C frame-drops=n/a vo-delayed=n/a"
        );
        // i64::MIN: the one value where a naive `.abs()` would panic.
        // `unsigned_abs()` does not.
        let s = format_heartbeat(0, 0, Some(i64::MIN), None, None);
        assert!(s.starts_with("dex-loop: heartbeat wraps=0 uptime=0s temp=-"), "{s}");
    }

    #[test]
    fn missing_sources_degrade_to_na_not_errors() {
        assert_eq!(
            format_heartbeat(0, 0, None, None, None),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=n/a frame-drops=n/a vo-delayed=n/a"
        );
    }
}
