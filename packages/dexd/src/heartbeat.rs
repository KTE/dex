//! The heartbeat line: one line in the system log every ten minutes, so a
//! failure at the venue is still diagnosable weeks later. Formatting lives
//! here and is unit-tested; the scheduling and the mpv and sysfs plumbing
//! live in main.rs.
//!
//! ```text
//! dexd: heartbeat loops=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=2 pos=3599.4s pos-age=0s watchdog=armed pings-dropped=0
//! ```
//!
//! Every value arrives in an `MPV_EVENT_PROPERTY_CHANGE` event. No function
//! here takes an mpv handle, which keeps a synchronous property read — a wait
//! on mpv's core thread with no timeout — out of the diagnostic.
//!
//! A value that has not arrived prints `n/a`; a counter whose subscription
//! never registered prints `off`, a different fact.
//!
//! See docs/design/failure-handling.md#heartbeat.

/// One mpv counter as the heartbeat sees it: a value learned from
/// `MPV_EVENT_PROPERTY_CHANGE` events alone, never from a synchronous read.
/// See docs/design/failure-handling.md#heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedCounter {
    observed: bool,
    last_raw: Option<u64>,
    /// Whether any real sample has arrived. Separate from `last_raw`, which
    /// `mark_unavailable` clears: a total already counted keeps rendering as
    /// a number across the few-second gap every in-place recovery leaves.
    has_sample: bool,
    total: u64,
}

impl ObservedCounter {
    /// Start a counter whose `mpv_observe_property` call succeeded, so this
    /// run can produce values.
    pub const fn observed() -> Self {
        Self {
            observed: true,
            last_raw: None,
            has_sample: false,
            total: 0,
        }
    }

    /// Start a counter whose `mpv_observe_property` call failed, so this run
    /// never produces a value. Renders `off`: the subscription is missing,
    /// where `n/a` means it is registered and still waiting for a first
    /// value.
    pub const fn unobserved() -> Self {
        Self {
            observed: false,
            last_raw: None,
            has_sample: false,
            total: 0,
        }
    }

    /// Feed one `MPV_EVENT_PROPERTY_CHANGE` payload, already unwrapped from
    /// `MPV_FORMAT_INT64`. Adds forward deltas and reads a decrease as mpv
    /// restarting the counter for a new playback session, so the total keeps
    /// climbing across an in-place recovery.
    /// See docs/design/failure-handling.md#heartbeat.
    pub fn sample(&mut self, raw: i64) {
        // mpv's drop counters count one playback session: an in-place
        // recovery (`loadfile ... replace`) rebuilds the video output chain
        // and restarts both at 0. Adding forward deltas, and treating any
        // decrease as a reset whose post-reset value is itself new, keeps the
        // printed total from falling back to `frame-drops=0` after a recovery
        // that dropped hundreds.
        //
        // `max(0)`: mpv reports no negative drop count, but this value
        // crosses a hand-written FFI boundary, and the clamp keeps a negative
        // from wrapping into billions as u64.
        let raw = raw.max(0) as u64;
        let delta = match self.last_raw {
            Some(prev) if raw >= prev => raw - prev,
            Some(_) => raw, // counter reset: everything visible now is new
            None => raw,    // first sample of the process
        };
        self.total = self.total.saturating_add(delta);
        self.last_raw = Some(raw);
        self.has_sample = true;
    }

    /// Cumulative count since process start, or `None` if no sample has
    /// arrived yet. Accessor for callers and tests; the heartbeat line goes
    /// through `Display`.
    pub fn total(&self) -> Option<u64> {
        self.has_sample.then_some(self.total)
    }

    /// Record that mpv reported this property as unavailable: an
    /// `MPV_EVENT_PROPERTY_CHANGE` carrying `format=MPV_FORMAT_NONE`, which
    /// arrives whenever no video output chain exists, at startup and during
    /// an in-place recovery.
    ///
    /// `total` is untouched; `last_raw` is cleared, so the next real sample
    /// counts in full instead of being diffed against a value from a playback
    /// session that has ended.
    /// See docs/design/failure-handling.md#heartbeat.
    pub fn mark_unavailable(&mut self) {
        self.last_raw = None;
    }
}

impl std::fmt::Display for ObservedCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.observed {
            // No subscription was registered, so no number is available; the
            // startup lines give the reason.
            write!(f, "off")
        } else if let Some(total) = self.total() {
            write!(f, "{total}")
        } else {
            // Registered, and no sample has arrived yet. Two causes render
            // the same way: no video output chain has come up in this run,
            // expected only for the first heartbeat; or the property name is
            // unknown to this mpv build, because `mpv_observe_property`
            // accepts a name it does not recognise and fails only on a bad
            // format or out of memory. A run that stays at `n/a` for hours
            // rather than seconds is itself the diagnosis: the subscription
            // registered and no value arrived.
            write!(f, "n/a")
        }
    }
}

/// The last known playback position and how stale it is. Both values arrive
/// together in the same `MPV_EVENT_PROPERTY_CHANGE`, so neither is ever known
/// without the other.
/// See docs/design/failure-handling.md#heartbeat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionSample {
    pub secs: f64,
    pub age_secs: u64,
}

/// Everything one heartbeat line prints.
/// See docs/design/failure-handling.md#heartbeat.
#[derive(Debug, Clone, Copy)]
pub struct HeartbeatSnapshot {
    pub loops: u64,
    pub uptime_secs: u64,
    pub temp_millicelsius: Option<i64>,
    pub position: Option<PositionSample>,
    pub frame_drops: ObservedCounter,
    pub vo_delayed: ObservedCounter,
    /// `None` when the systemd watchdog is inert for this run (see
    /// `watchdog::InertReason` for the causes); `Some(n)` when it is armed,
    /// `n` counting the pings that could not be sent since process start.
    pub watchdog_pings_dropped: Option<u64>,
}

impl HeartbeatSnapshot {
    /// Render one heartbeat line.
    pub fn render(&self) -> String {
        let temp = match self.temp_millicelsius {
            Some(m) => {
                // Take the sign before dividing: `m / 1000` truncates toward
                // zero, so any value in -999..=-1 divides to 0 and would
                // print as a positive "0.2C". `unsigned_abs` also avoids the
                // one panic in this shape, `i64::MIN.abs()`.
                let (sign, mag) = if m < 0 {
                    ("-", m.unsigned_abs())
                } else {
                    ("", m as u64)
                };
                format!("{sign}{}.{}C", mag / 1000, (mag % 1000) / 100)
            }
            None => "n/a".to_string(),
        };
        let (pos, pos_age) = match self.position {
            Some(PositionSample { secs, age_secs }) => {
                (format!("{secs:.1}s"), format!("{age_secs}s"))
            }
            None => ("n/a".to_string(), "n/a".to_string()),
        };
        // `inert` and `armed pings-dropped=0` are different facts: no systemd
        // watchdog at all, against one that is armed and has lost no ping.
        // Same split as `off` against `n/a` above.
        let watchdog = match self.watchdog_pings_dropped {
            Some(n) => format!("armed pings-dropped={n}"),
            None => "inert".to_string(),
        };
        format!(
            "dexd: heartbeat loops={} uptime={}s temp={temp} frame-drops={} \
             vo-delayed={} pos={pos} pos-age={pos_age} watchdog={watchdog}",
            self.loops, self.uptime_secs, self.frame_drops, self.vo_delayed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ObservedCounter ----------------------------------------------

    #[test]
    fn unobserved_renders_off_even_after_a_stray_sample() {
        let mut c = ObservedCounter::unobserved();
        assert_eq!(c.to_string(), "off");
        // Nothing feeds an unobserved counter in practice. The render stays
        // `off` regardless, because `off` means the subscription never
        // registered and a later sample does not change that.
        c.sample(5);
        assert_eq!(c.to_string(), "off");
    }

    #[test]
    fn observed_with_no_sample_renders_na() {
        assert_eq!(ObservedCounter::observed().to_string(), "n/a");
        assert_eq!(ObservedCounter::observed().total(), None);
    }

    #[test]
    fn first_sample_becomes_the_total() {
        let mut c = ObservedCounter::observed();
        c.sample(7);
        assert_eq!(c.total(), Some(7));
        assert_eq!(c.to_string(), "7");
    }

    #[test]
    fn monotone_samples_accumulate_by_delta() {
        let mut c = ObservedCounter::observed();
        c.sample(2);
        c.sample(5);
        c.sample(5); // no change: delta 0
        c.sample(9);
        assert_eq!(c.total(), Some(9));
    }

    #[test]
    fn a_decrease_is_a_recovery_reset_and_the_post_reset_value_is_added() {
        // An in-place recovery (`loadfile ... replace`) rebuilds the video
        // output chain, and mpv restarts frame-drop-count at 0 for the new
        // playback session. The heartbeat keeps counting forward instead.
        let mut c = ObservedCounter::observed();
        c.sample(40); // 40 drops accumulated before the recovery
        c.sample(3); // the new playback session's counter, already at 3
        assert_eq!(
            c.total(),
            Some(43),
            "the post-reset value must be added to the total, not replace it"
        );
        c.sample(3); // steady after the reset: no further change
        assert_eq!(c.total(), Some(43));
    }

    #[test]
    fn mark_unavailable_keeps_total_but_makes_the_next_sample_a_fresh_first_sample() {
        let mut c = ObservedCounter::observed();
        c.sample(40);
        assert_eq!(c.total(), Some(40));

        c.mark_unavailable();
        assert_eq!(
            c.total(),
            Some(40),
            "a counted total keeps rendering as a number after \
             mark_unavailable: total() is gated on has_sample, not on the \
             diffing baseline that mark_unavailable clears"
        );

        c.sample(7);
        assert_eq!(
            c.total(),
            Some(47),
            "a sample after mark_unavailable counts in full, instead of being \
             diffed against the cleared last_raw"
        );
    }

    #[test]
    fn negative_raw_is_clamped_to_zero() {
        let mut c = ObservedCounter::observed();
        c.sample(-1);
        assert_eq!(c.total(), Some(0));
    }

    #[test]
    fn total_saturates_at_u64_max() {
        // Three `i64::MAX` contributions overflow u64, since u64::MAX is
        // 2 * i64::MAX + 1. Each contribution arrives as a "reset to 0, then
        // jump back to i64::MAX" pair, so the test exercises `saturating_add`
        // while every value that crosses the FFI boundary stays a legal i64.
        let mut c = ObservedCounter::observed();
        c.sample(i64::MAX);
        c.sample(0); // reset
        c.sample(i64::MAX);
        c.sample(0); // reset
        c.sample(i64::MAX);
        assert_eq!(c.total(), Some(u64::MAX));
    }

    // --- HeartbeatSnapshot::render --------------------------------------

    #[test]
    fn renders_the_full_line() {
        let mut frame_drops = ObservedCounter::observed();
        frame_drops.sample(0);
        let mut vo_delayed = ObservedCounter::observed();
        vo_delayed.sample(2);
        let snap = HeartbeatSnapshot {
            loops: 143,
            uptime_secs: 3600,
            temp_millicelsius: Some(48_250),
            position: Some(PositionSample {
                secs: 3599.4,
                age_secs: 0,
            }),
            frame_drops,
            vo_delayed,
            watchdog_pings_dropped: Some(0),
        };
        assert_eq!(
            snap.render(),
            "dexd: heartbeat loops=143 uptime=3600s temp=48.2C frame-drops=0 \
             vo-delayed=2 pos=3599.4s pos-age=0s watchdog=armed pings-dropped=0"
        );
    }

    #[test]
    fn every_missing_source_renders_na() {
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: None,
        };
        assert_eq!(
            snap.render(),
            "dexd: heartbeat loops=0 uptime=0s temp=n/a frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a watchdog=inert"
        );
    }

    #[test]
    fn unregistered_counters_render_off() {
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::unobserved(),
            vo_delayed: ObservedCounter::unobserved(),
            watchdog_pings_dropped: None,
        };
        assert_eq!(
            snap.render(),
            "dexd: heartbeat loops=0 uptime=0s temp=n/a frame-drops=off \
             vo-delayed=off pos=n/a pos-age=n/a watchdog=inert"
        );
    }

    #[test]
    fn armed_watchdog_prints_the_dropped_ping_count() {
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: Some(7),
        };
        assert!(
            snap.render().ends_with("watchdog=armed pings-dropped=7"),
            "{}",
            snap.render()
        );
    }

    #[test]
    fn inert_watchdog_never_prints_a_dropped_count() {
        // `None` means there is no systemd watchdog at all, a different fact
        // from armed with zero drops so far, which `renders_the_full_line`
        // covers as `Some(0)`.
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: None,
        };
        let s = snap.render();
        assert!(s.ends_with("watchdog=inert"), "{s}");
        assert!(!s.contains("pings-dropped"), "{s}");
    }

    #[test]
    fn sub_zero_temperatures_keep_their_sign() {
        // Dividing before taking the sign gives `m / 1000 == 0` for any m in
        // -999..=-1, so -250 m°C would render as the positive "0.2C".
        let base = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: Some(-250),
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: None,
        };
        assert_eq!(
            base.render(),
            "dexd: heartbeat loops=0 uptime=0s temp=-0.2C frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a watchdog=inert"
        );
        let mut base = base;
        base.temp_millicelsius = Some(-1_500);
        assert_eq!(
            base.render(),
            "dexd: heartbeat loops=0 uptime=0s temp=-1.5C frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a watchdog=inert"
        );
        // i64::MIN: the one value a plain `.abs()` would panic on.
        // `unsigned_abs()` does not.
        base.temp_millicelsius = Some(i64::MIN);
        let s = base.render();
        assert!(s.contains("temp=-"), "{s}");
    }

    #[test]
    fn position_formats_to_one_decimal() {
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: Some(PositionSample {
                secs: 12.049,
                age_secs: 4,
            }),
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: None,
        };
        assert!(
            snap.render().contains("pos=12.0s pos-age=4s"),
            "{}",
            snap.render()
        );
    }

    #[test]
    fn a_position_after_weeks_of_uptime_stays_in_decimal_notation() {
        // Three weeks at about realtime rate: past the point where `{}` would
        // switch a float to scientific notation, which the fixed `{:.1}`
        // format in `render` prevents.
        let three_weeks_secs = 3600.0 * 24.0 * 21.0;
        let snap = HeartbeatSnapshot {
            loops: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: Some(PositionSample {
                secs: three_weeks_secs,
                age_secs: 0,
            }),
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
            watchdog_pings_dropped: None,
        };
        let s = snap.render();
        assert!(s.contains("pos=1814400.0s"), "{s}");
        // Scoped to the pos field alone, because "heartbeat" itself
        // contains an 'e'.
        let pos_field = s.split("pos=").nth(1).unwrap().split(' ').next().unwrap();
        assert!(
            !pos_field.contains('e') && !pos_field.contains('E'),
            "{pos_field}"
        );
    }
}
