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
        Some(m) => format!("{}.{}C", m / 1000, (m % 1000).abs() / 100),
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
    fn missing_sources_degrade_to_na_not_errors() {
        assert_eq!(
            format_heartbeat(0, 0, None, None, None),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=n/a frame-drops=n/a vo-delayed=n/a"
        );
    }
}
