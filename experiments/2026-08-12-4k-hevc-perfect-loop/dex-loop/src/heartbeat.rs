//! F7/F9 — the heartbeat line: one log line every ~10 min so a weeks-later
//! field failure is diagnosable from the journal after the fact (was it
//! degrading? hot? dropping frames? did the core stop answering?).
//! Formatting is pure and unit-tested here; scheduling and the mpv/sysfs
//! plumbing live in main.rs.
//!
//! # F9 — why every field here comes from an event, never a synchronous read
//!
//! Before F9, `emit_heartbeat` called `mpv_get_property_string` directly from
//! the event thread. Verified against mpv v0.40.0 source:
//! `mpv_get_property_string` -> `run_locked` -> `mp_dispatch_lock`
//! (misc/dispatch.c:364-394) spins on `mp_cond_wait` with **no timeout**
//! until the core thread is trapped inside `mp_dispatch_queue_process()`. A
//! core thread wedged in a DRM ioctl mid-playloop never reaches that trap
//! point, so the caller -- this event thread, the same one `mpv_wait_event`
//! runs on -- blocks forever. That is bug #1's exact shape (process alive,
//! supervisor green, screen black) entered through the one diagnostic that
//! was supposed to help detect it.
//!
//! The fix is not "read less often", it is "never call in". Every value a
//! heartbeat line prints now arrives ONLY via `MPV_EVENT_PROPERTY_CHANGE`,
//! the same door F1's `time-pos` subscription already uses. Three source-backed
//! properties make that strictly better than the synchronous read it
//! replaces, not merely no-worse:
//!
//! 1. **The read migrates off our thread.** Observed-property getters run on
//!    the core thread via `send_client_property_changes()`, which explicitly
//!    drops the client lock around the getter call (player/client.c
//!    :1694-1699, "property getters can do whatever they want"). A wedged
//!    getter blocks neither our event thread nor `mpv_wait_event`. If the
//!    core is wedged we get silence, not a hang -- and silence is exactly
//!    the signal `pos-age=` below exists to surface.
//! 2. **Steady-state cost is one event per counter, ever.** Change events
//!    fire only when the value actually changes (`equal_mpv_value`,
//!    player/client.c:1715-1717), plus one guaranteed initial notification.
//!    A gallery run with no drops emits each counter once at startup, then
//!    silence for three weeks.
//! 3. **They cannot contribute to `QUEUE_OVERFLOW`.** Property-change events
//!    are "never queued" (player/client.c:942-943) -- generated inside
//!    `mpv_wait_event` only once the queue has drained. Observing more
//!    properties cannot push this program toward the one event it treats as
//!    fatal.
//!
//! `mpv_get_property_string` and the `mpv_free` it requires are gone from
//! main.rs's FFI surface entirely (not merely unused) -- see the comment
//! left in their place there. With no `*mut MpvHandle` in scope,
//! `emit_heartbeat` cannot call into mpv even by accident. That is a gate,
//! not a rule: per this repo's gates-over-rules principle, "don't add a
//! blocking read here" as a comment can be forgotten; a missing parameter
//! cannot.

/// One mpv counter as the heartbeat knows it: a value learned ONLY from
/// `MPV_EVENT_PROPERTY_CHANGE`, never from a synchronous read (see the
/// module doc for why that distinction is the whole of F9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedCounter {
    observed: bool,
    last_raw: Option<u64>,
    total: u64,
}

impl ObservedCounter {
    /// `mpv_observe_property` succeeded: this run can produce values.
    pub const fn observed() -> Self {
        Self { observed: true, last_raw: None, total: 0 }
    }

    /// `mpv_observe_property` FAILED: this run never will. Renders `off`,
    /// which is a different fact from "no value yet" -- see the module doc
    /// (principle 2, "distinguish the two no-value cases").
    pub const fn unobserved() -> Self {
        Self { observed: false, last_raw: None, total: 0 }
    }

    /// Feed one `MPV_EVENT_PROPERTY_CHANGE` payload (already unwrapped from
    /// `MPV_FORMAT_INT64`). Accumulates across per-file counter resets --
    /// see the body comment for why that is required, not merely tidy.
    pub fn sample(&mut self, raw: i64) {
        // mpv's drop counters are PER PLAYBACK SESSION: F1's tier-0 recovery
        // (`loadfile ... replace`) rebuilds the VO chain and restarts both
        // at 0. Printed raw, the last heartbeat before a field failure could
        // read `frame-drops=0` purely because a recovery reset it four
        // minutes earlier -- a false all-clear in the ONE line that is the
        // only diagnostic channel on site. So accumulate: add forward
        // deltas, and treat any decrease as a reset whose post-reset value
        // is itself new.
        //
        // `max(0)`: mpv never reports a negative drop count, but this value
        // arrives through a hand-transcribed FFI tag check, and a clamp is
        // cheaper than a cast that could wrap into billions.
        let raw = raw.max(0) as u64;
        let delta = match self.last_raw {
            Some(prev) if raw >= prev => raw - prev,
            Some(_) => raw, // counter reset: everything visible now is new
            None => raw,    // first sample of the process
        };
        self.total = self.total.saturating_add(delta);
        self.last_raw = Some(raw);
    }

    /// Cumulative count since process start, or `None` if no sample has
    /// arrived yet. Test/caller accessor; rendering goes through `Display`.
    pub fn total(&self) -> Option<u64> {
        self.last_raw.map(|_| self.total)
    }
}

impl std::fmt::Display for ObservedCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.observed {
            // Subscription never registered: this number is not evidence of
            // anything, look at the startup lines instead.
            write!(f, "off")
        } else if let Some(total) = self.total() {
            write!(f, "{total}")
        } else {
            // Registered, but no value has arrived yet (heartbeat #0, or no
            // VO chain exists). A run stuck here for hours IS the
            // diagnosis: the core asked and never answered.
            write!(f, "n/a")
        }
    }
}

/// The last known playback position and how stale it is. One struct, not
/// two `Option`s, because "position known but age unknown" cannot occur --
/// both come from the same `MPV_EVENT_PROPERTY_CHANGE`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionSample {
    pub secs: f64,
    pub age_secs: u64,
}

/// Everything one heartbeat line prints. A struct with NAMED fields, not a
/// multi-argument function: two of the fields are `ObservedCounter` and two
/// are integers, so a positional call site could silently transpose
/// frame_drops/vo_delayed (or wraps/uptime) and no test in the crate would
/// catch it -- main.rs is the one file the Mac cannot run.
#[derive(Debug, Clone, Copy)]
pub struct HeartbeatSnapshot {
    pub wraps: u64,
    pub uptime_secs: u64,
    pub temp_millicelsius: Option<i64>,
    pub position: Option<PositionSample>,
    pub frame_drops: ObservedCounter,
    pub vo_delayed: ObservedCounter,
}

impl HeartbeatSnapshot {
    /// Render one heartbeat line.
    pub fn render(&self) -> String {
        let temp = match self.temp_millicelsius {
            Some(m) => {
                // Extract the sign before dividing: `m / 1000` truncates
                // toward zero, so for m in -999..=-1 it is 0 -- an unheated
                // venue on a winter cold-boot (e.g. -250 m°C) would
                // otherwise render as a plausible-looking POSITIVE "0.2C" in
                // the journal. `unsigned_abs` sidesteps the one panic hazard
                // in this shape (`i64::MIN.abs()`) entirely, though sysfs
                // never reports a value near it.
                let (sign, mag) = if m < 0 { ("-", m.unsigned_abs()) } else { ("", m as u64) };
                format!("{sign}{}.{}C", mag / 1000, (mag % 1000) / 100)
            }
            None => "n/a".to_string(),
        };
        let (pos, pos_age) = match self.position {
            Some(PositionSample { secs, age_secs }) => (format!("{secs:.1}s"), format!("{age_secs}s")),
            None => ("n/a".to_string(), "n/a".to_string()),
        };
        format!(
            "dex-loop: heartbeat wraps={} uptime={}s temp={temp} frame-drops={} \
             vo-delayed={} pos={pos} pos-age={pos_age}",
            self.wraps, self.uptime_secs, self.frame_drops, self.vo_delayed,
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
        // A stray sample must not happen in practice (nothing feeds an
        // unobserved counter), but the render must stay "off" regardless --
        // "off" means "the subscription never registered", which a sample
        // arriving cannot retroactively change.
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
    fn a_decrease_is_a_tier_0_recovery_reset_and_accumulates_the_post_reset_value() {
        // The scenario this pins: F1's in-place recovery (`loadfile ...
        // replace`) rebuilds the VO chain, and mpv restarts frame-drop-count
        // at 0 for the new playback session. Printed raw, that reset would
        // make `frame-drops=0` look like a false all-clear right after a
        // recovery. The heartbeat must instead keep counting forward from
        // where it was.
        let mut c = ObservedCounter::observed();
        c.sample(40); // pre-recovery: 40 drops accumulated
        c.sample(3); // recovery reset the session counter to 3
        assert_eq!(c.total(), Some(43), "the post-reset value must be added, not replace the total");
        c.sample(3); // steady after the reset: no further change
        assert_eq!(c.total(), Some(43));
    }

    #[test]
    fn negative_raw_is_clamped_to_zero() {
        let mut c = ObservedCounter::observed();
        c.sample(-1);
        assert_eq!(c.total(), Some(0));
    }

    #[test]
    fn total_saturates_at_u64_max() {
        // Three `i64::MAX` contributions overflow u64 (u64::MAX = 2 *
        // i64::MAX + 1, so a third addition of i64::MAX always overflows).
        // Each contribution arrives via a "reset to 0, then jump back to
        // i64::MAX" pair so every sample stays a legal i64 input -- this
        // tests `saturating_add`, not a way to smuggle an out-of-range raw
        // value past the FFI boundary.
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
            wraps: 143,
            uptime_secs: 3600,
            temp_millicelsius: Some(48_250),
            position: Some(PositionSample { secs: 3599.4, age_secs: 0 }),
            frame_drops,
            vo_delayed,
        };
        assert_eq!(
            snap.render(),
            "dex-loop: heartbeat wraps=143 uptime=3600s temp=48.2C frame-drops=0 \
             vo-delayed=2 pos=3599.4s pos-age=0s"
        );
    }

    #[test]
    fn all_missing_sources_degrade_to_na_not_errors() {
        let snap = HeartbeatSnapshot {
            wraps: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
        };
        assert_eq!(
            snap.render(),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=n/a frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a"
        );
    }

    #[test]
    fn unregistered_counters_render_off() {
        let snap = HeartbeatSnapshot {
            wraps: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: None,
            frame_drops: ObservedCounter::unobserved(),
            vo_delayed: ObservedCounter::unobserved(),
        };
        assert_eq!(
            snap.render(),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=n/a frame-drops=off \
             vo-delayed=off pos=n/a pos-age=n/a"
        );
    }

    #[test]
    fn sub_zero_temperatures_keep_their_sign() {
        // -250 m°C: an unheated venue on a winter cold-boot. Before the
        // original fix, `m / 1000 == 0` for any m in -999..=-1, so this
        // silently rendered as the POSITIVE "0.2C" -- a plausible-looking
        // wrong reading in exactly the diagnostic line this module exists
        // to make trustworthy. Carried over verbatim from F7.
        let base = HeartbeatSnapshot {
            wraps: 0,
            uptime_secs: 0,
            temp_millicelsius: Some(-250),
            position: None,
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
        };
        assert_eq!(
            base.render(),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=-0.2C frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a"
        );
        let mut base = base;
        base.temp_millicelsius = Some(-1_500);
        assert_eq!(
            base.render(),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=-1.5C frame-drops=n/a \
             vo-delayed=n/a pos=n/a pos-age=n/a"
        );
        // i64::MIN: the one value where a naive `.abs()` would panic.
        // `unsigned_abs()` does not.
        base.temp_millicelsius = Some(i64::MIN);
        let s = base.render();
        assert!(s.contains("temp=-"), "{s}");
    }

    #[test]
    fn position_formats_to_one_decimal() {
        let snap = HeartbeatSnapshot {
            wraps: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: Some(PositionSample { secs: 12.049, age_secs: 4 }),
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
        };
        assert!(snap.render().contains("pos=12.0s pos-age=4s"), "{}", snap.render());
    }

    #[test]
    fn a_large_position_from_weeks_of_uptime_does_not_go_exponential() {
        // Three weeks at ~1x realtime: well past the point where `{}` would
        // switch a float to scientific notation if this used a bare
        // Display impl instead of a fixed `{:.1}` format.
        let three_weeks_secs = 3600.0 * 24.0 * 21.0;
        let snap = HeartbeatSnapshot {
            wraps: 0,
            uptime_secs: 0,
            temp_millicelsius: None,
            position: Some(PositionSample { secs: three_weeks_secs, age_secs: 0 }),
            frame_drops: ObservedCounter::observed(),
            vo_delayed: ObservedCounter::observed(),
        };
        let s = snap.render();
        assert!(s.contains("pos=1814400.0s"), "{s}");
        // Not a whole-line check: "heartbeat" itself contains 'e'. Scope the
        // scientific-notation check to the pos field's rendered text alone.
        let pos_field = s.split("pos=").nth(1).unwrap().split(' ').next().unwrap();
        assert!(!pos_field.contains('e') && !pos_field.contains('E'), "{pos_field}");
    }
}
