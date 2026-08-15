//! F1 — tier-0 self-healing: the escalation policy for the periodic health
//! check, extracted pure so the policy that decides "recover in place" vs.
//! "give up and let the supervisor restart" is testable without libmpv, a
//! display, or even the crate's FFI half. `main.rs` is a thin driver over
//! this: it samples a playback-position property on a fixed cadence and
//! feeds the sample to [`HealthMonitor::tick`]; every other decision is made
//! here.
//!
//! # Why this exists (PLAN.md F1)
//!
//! The event loop already treats a fatal mpv event (END_FILE,
//! QUEUE_OVERFLOW) as tier 1: exit non-zero, let `Restart=always` recover.
//! That covers a display absent at BOOT. It does not cover an INTERMITTENT
//! loss mid-show — the projector's HDMI blinking, a sink waking up late —
//! where mpv itself never emits a fatal event at all: it just silently stops
//! making progress while the process stays alive and every existing signal
//! (heartbeat, supervisor) reads green. Killing a player that LOOKS healthy
//! over one glitch is heavy-handed and loses seconds of picture in front of
//! the public; never checking at all is bug #1's exact shape (alive, green,
//! screen black) with a different trigger. Tier 0 is the middle path: try a
//! cheap in-place repair first, and escalate only if that keeps failing.
//!
//! # The escalation policy, precisely
//!
//! * A "tick" happens on a fixed wall-clock cadence (main.rs: ~10 s, off the
//!   decode path — see that module's comment on the event-wait timeout for
//!   why the cadence has to be enforced by a TIMEOUT, not just by events).
//! * Each tick reports the LATEST known playback position (e.g. `time-pos`).
//!   `None` means no update has arrived since the monitor was created (or
//!   since it was last reset by a recovery) — treated identically to "the
//!   value is unchanged", because a wedged core stops producing property
//!   updates entirely; the absence of an update IS the stall symptom, not a
//!   distinct case needing its own handling.
//! * The position must strictly increase between two ticks to count as
//!   progress. Two CONSECUTIVE non-advancing ticks (not one) are required
//!   before acting, to absorb ordinary jitter around a check boundary — see
//!   PLAN.md's F1 text ("has not advanced across two consecutive checks").
//! * Once that bar is met, the caller is told to attempt an in-place
//!   recovery (re-issue `loadfile ... replace`, which opens a brand-new
//!   `loop://` stream and forces mpv to reconfigure the VO).
//! * Recovery attempts are drawn from a budget fixed at construction and
//!   NEVER replenished for the life of the process — see "why the budget
//!   never resets" below. Once exhausted, the next qualifying stall
//!   escalates instead of attempting another recovery.
//!
//! # Why the budget never resets on a temporary recovery
//!
//! The obvious alternative — give the counter back after some period of
//! sustained health following a recovery — reopens exactly the hole the cap
//! exists to close. A fault that FLAPS (heals for a while, stalls again,
//! repeat) would keep resetting the counter before it ever reached the cap,
//! producing a total number of in-place retries that is unbounded across the
//! process's lifetime even though each individual episode looks bounded.
//! That is "an unbounded in-place retry loop... wearing a different hat" —
//! exactly the failure mode the task brief warns against, just spread out in
//! time instead of packed into one burst. A cumulative, never-replenished
//! budget is the only shape that bounds the WORST case, not just the common
//! one. And leaning on tier 1 sooner than strictly necessary is cheap and
//! safe here: `Restart=always` with `StartLimitIntervalSec=0`
//! (`deploy/dex-loop.service`) never gives up either, so the process comes
//! back regardless of which tier does the healing — tier 0 running out of
//! budget means tier 1 takes over, not that the show stops.
//!
//! # Why the sample that drives this arrives asynchronously (F9's concern)
//!
//! A prior review (F9, SUSPECTED, recorded in PLAN.md) flagged that a
//! synchronous `mpv_get_property_string` call made from a supervisor thread
//! is itself a core-wedge risk: if the mpv core is ever stuck (e.g. the VO
//! thread blocked in a DRM ioctl against a dying projector, holding whatever
//! the core needs), a synchronous property read could block that thread
//! forever — reproducing bug #1's exact shape (alive, supervisor green,
//! screen black) through a new door instead of closing it. F1 takes that
//! seriously rather than reproducing it:
//!
//! * `main.rs` never polls `mpv_get_property_string` (or any synchronous
//!   property read) for this feature. It registers exactly ONE
//!   `mpv_observe_property` call at startup — documented in client.h as
//!   non-blocking, queuing a subscription and returning immediately — and
//!   thereafter only ever learns the position from `MPV_EVENT_PROPERTY_CHANGE`
//!   events delivered through the SAME `mpv_wait_event` loop that already
//!   proves, by the very fact that this program correctly detects END_FILE
//!   and QUEUE_OVERFLOW today, that it cannot block indefinitely (the wait
//!   call takes a caller-supplied timeout).
//! * If the core wedges, no new property-change events arrive at all. That
//!   surfaces here as `tick(None)` (or an unchanging value) — exactly the
//!   stall signal this module already exists to notice, produced by the same
//!   mechanism that already detects every other fault, rather than by a new
//!   blocking call that could itself hang.
//! * Recovery is issued the same way: `mpv_command_async`, not the
//!   synchronous `mpv_command` this program already uses once at startup
//!   (safe there — nothing has had a chance to wedge before the first
//!   frame). Using the async variant for recovery means the one new
//!   synchronous-shaped risk this feature could have introduced — blocking
//!   on `loadfile` from the event thread while trying to fix a wedged core —
//!   does not exist either.
//!
//! In short: every new mpv-facing call this feature adds is either
//! documented non-blocking (`mpv_observe_property`, `mpv_command_async`) or
//! not a new call at all (`mpv_wait_event`, already relied on). No new
//! synchronous call is introduced, so F1 does not add a new way to hang —
//! the exact requirement the task brief states.

/// What the caller should do after feeding one health-check sample to
/// [`HealthMonitor::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAction {
    /// Progress since the last tick, or too early to judge yet (the very
    /// first sample, or only one non-advancing tick so far). Nothing to do.
    Healthy,
    /// No progress across >= 2 consecutive ticks, and the recovery budget is
    /// not yet exhausted. The caller should force VO reconfiguration
    /// (re-issue `loadfile ... replace`, asynchronously) and keep running.
    AttemptRecovery {
        /// This attempt's number (1-based) against `max`.
        attempt: u32,
        max: u32,
    },
    /// No progress across >= 2 consecutive ticks, and the recovery budget is
    /// exhausted. The caller should escalate to tier 1: exit non-zero so the
    /// supervisor restarts the whole process.
    Escalate,
}

/// The tier-0 escalation state machine. See the module doc for the policy
/// and the reasoning behind it; this struct only holds the state that
/// policy needs.
pub struct HealthMonitor {
    last_position: Option<f64>,
    consecutive_stalls: u32,
    recovery_attempts_used: u32,
    max_recovery_attempts: u32,
}

impl HealthMonitor {
    /// `max_recovery_attempts` is the CUMULATIVE, process-lifetime budget of
    /// in-place recoveries — see the module doc for why it never refills.
    pub fn new(max_recovery_attempts: u32) -> Self {
        Self {
            last_position: None,
            consecutive_stalls: 0,
            recovery_attempts_used: 0,
            max_recovery_attempts,
        }
    }

    /// Feed one health-check tick. `position` is the latest known playback
    /// position (e.g. `time-pos` seconds), or `None` if no update has been
    /// observed since the monitor was created, or since it was last reset by
    /// a recovery attempt — see the module doc for why that is treated the
    /// same as "value unchanged".
    pub fn tick(&mut self, position: Option<f64>) -> HealthAction {
        let advanced = match (position, self.last_position) {
            (Some(p), Some(prev)) => p > prev,
            // First sample since start (or since the last recovery reset
            // the baseline below): nothing to compare against yet. Treating
            // this as progress, not a stall, avoids counting mpv's own
            // startup/reload latency -- a few hundred ms to a few seconds
            // for a 4K decode to begin -- as a fault. A REAL startup hang
            // (no sample ever arrives) is still caught: two consecutive
            // `None` ticks below is exactly that case.
            (Some(_), None) => true,
            (None, _) => false,
        };

        if let Some(p) = position {
            self.last_position = Some(p);
        }

        if advanced {
            self.consecutive_stalls = 0;
            return HealthAction::Healthy;
        }

        self.consecutive_stalls += 1;
        if self.consecutive_stalls < 2 {
            return HealthAction::Healthy;
        }

        // Two consecutive non-advancing ticks: a qualifying stall episode.
        // Reset the LOCAL jitter counter so the next attempt (if any) or the
        // eventual escalation gets its own clean 2-tick window rather than
        // re-triggering on the very next tick -- but do NOT reset the
        // budget itself; see the module doc.
        self.consecutive_stalls = 0;

        if self.recovery_attempts_used >= self.max_recovery_attempts {
            return HealthAction::Escalate;
        }
        self.recovery_attempts_used += 1;
        // A successful in-place recovery re-opens the stream from byte 0
        // (main.rs issues `loadfile ... replace` on the SAME loop:// URL,
        // which creates a brand-new stream via open_fn), so it legitimately
        // restarts mpv's own position counter near zero. Forget the
        // pre-recovery baseline so the very next real sample -- however
        // small -- reads as progress rather than "still less than the old
        // high-water mark", which would otherwise misread a recovery that
        // WORKED as a continuing stall.
        self.last_position = None;
        HealthAction::AttemptRecovery {
            attempt: self.recovery_attempts_used,
            max: self.max_recovery_attempts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_ever_tick_is_healthy_even_with_no_prior_position() {
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(0.0)), HealthAction::Healthy);
    }

    #[test]
    fn steady_advancement_never_acts() {
        let mut m = HealthMonitor::new(3);
        let mut pos = 0.0;
        for _ in 0..50 {
            pos += 0.033; // roughly one 30fps frame's worth per tick
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }
    }

    #[test]
    fn a_single_non_advancing_tick_is_not_enough_to_act() {
        // PLAN.md: "if it has NOT ADVANCED ACROSS TWO CONSECUTIVE checks" --
        // one stalled tick must not trigger anything, or ordinary jitter
        // around a check boundary would cause spurious recoveries.
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy); // stall #1
        assert_eq!(m.tick(Some(2.0)), HealthAction::Healthy); // recovers before 2
    }

    #[test]
    fn two_consecutive_stalls_trigger_the_first_recovery_attempt() {
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy); // stall #1
        assert_eq!(
            m.tick(Some(1.0)), // stall #2 -- qualifies
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn none_samples_count_as_stalls_exactly_like_an_unchanged_value() {
        // No property-change event arriving at all is the same symptom as
        // the value not changing -- see the module doc's F9 discussion.
        // Unlike a genuine `Some` value, `None` gets NO "first sample" grace
        // period: the forgiveness for `Some` exists to absorb ordinary
        // startup LATENCY (a value arrives a moment late), but a string of
        // `None`s is exactly the real-startup-hang case the module doc
        // calls out ("a value that never arrives at all") -- so it must not
        // be forgiven for an extra tick the way a merely-late value is.
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(None), HealthAction::Healthy); // stall #1
        assert_eq!(
            m.tick(None), // stall #2 -- qualifies, no grace period for None
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn recovery_gets_a_fresh_two_tick_window_before_being_judged_again() {
        let mut m = HealthMonitor::new(3);
        m.tick(Some(1.0));
        m.tick(Some(1.0)); // stall #1
        assert_eq!(
            m.tick(Some(1.0)), // stall #2: attempt 1
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // Immediately after an attempt, ONE more sample must NOT
        // re-trigger -- the attempt needs its own 2-tick window, otherwise
        // a recovery that has not even had time to take effect gets judged
        // as having already failed.
        assert_eq!(m.tick(Some(0.05)), HealthAction::Healthy);
    }

    #[test]
    fn recovery_resets_the_position_baseline_so_a_reload_restart_is_not_misread_as_still_stalled(
    ) {
        // loadfile replace opens a BRAND NEW loop:// stream, so a real
        // recovery legitimately restarts mpv's own position counter near
        // zero. Without resetting the monitor's baseline too, the very next
        // real sample (small) would compare as "less than" the
        // pre-recovery high-water mark and register as ANOTHER stall --
        // punishing the exact recovery that worked.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(500.0));
        m.tick(Some(500.0)); // stall #1
        assert_eq!(
            m.tick(Some(500.0)), // stall #2: recovery issued, baseline dropped
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // The reload starts over near zero -- this must read as progress,
        // not as "0.1 < 500, another stall".
        assert_eq!(m.tick(Some(0.1)), HealthAction::Healthy);
        assert_eq!(m.tick(Some(0.2)), HealthAction::Healthy);
    }

    #[test]
    fn a_healed_stream_stays_healthy_and_does_not_consume_further_budget() {
        let mut m = HealthMonitor::new(3);
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(
            m.tick(Some(1.0)),
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // The stream advances again (recovery worked): healthy indefinitely.
        let mut pos = 0.0;
        for _ in 0..20 {
            pos += 0.1;
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }
    }

    #[test]
    fn budget_is_cumulative_and_never_refills_across_separate_episodes() {
        // The behaviour the module doc's "why the budget never resets"
        // section defends: a fault that heals in between episodes must NOT
        // get its budget back, or a flapping fault produces an unbounded
        // total number of recovery attempts spread across many separate
        // episodes -- the same failure the cap exists to prevent, just
        // spread out in time instead of packed into one burst.
        let mut m = HealthMonitor::new(2);

        // Episode 1: stalls, recovers (attempt 1/2).
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(
            m.tick(Some(1.0)),
            HealthAction::AttemptRecovery { attempt: 1, max: 2 }
        );
        // Fully healthy for a long stretch afterwards.
        let mut pos = 0.0;
        for _ in 0..100 {
            pos += 0.1;
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }

        // Episode 2: stalls again, recovers (attempt 2/2 -- budget exhausted).
        let stalled_at = pos;
        m.tick(Some(stalled_at));
        assert_eq!(
            m.tick(Some(stalled_at)),
            HealthAction::AttemptRecovery { attempt: 2, max: 2 }
        );
        // Healthy again for a long stretch.
        pos = 0.0;
        for _ in 0..100 {
            pos += 0.1;
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }

        // Episode 3: budget is gone -- this one must escalate, not attempt
        // a third recovery, even though the stream has been perfectly
        // healthy for hundreds of ticks since the last stall.
        let stalled_at = pos;
        m.tick(Some(stalled_at));
        assert_eq!(m.tick(Some(stalled_at)), HealthAction::Escalate);
    }

    #[test]
    fn exhausting_the_budget_escalates_every_qualifying_stall_after() {
        let mut m = HealthMonitor::new(1);
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(
            m.tick(Some(1.0)),
            HealthAction::AttemptRecovery { attempt: 1, max: 1 }
        );
        // Budget is now exhausted (1/1 used). The next qualifying stall
        // must escalate, not attempt a second recovery.
        m.tick(Some(0.1));
        m.tick(Some(0.1));
        assert_eq!(m.tick(Some(0.1)), HealthAction::Escalate);
    }

    #[test]
    fn zero_budget_escalates_on_the_first_qualifying_stall() {
        // Edge case sanity: a monitor configured with NO recovery budget at
        // all still requires 2 consecutive stalls (the jitter guard is
        // unconditional) but then escalates immediately rather than ever
        // attempting an in-place recovery.
        let mut m = HealthMonitor::new(0);
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(m.tick(Some(1.0)), HealthAction::Escalate);
    }

    #[test]
    fn position_going_backwards_counts_as_a_stall_not_progress() {
        // Should never happen for a monotonic, clock-driven time-pos, but
        // the policy must not treat it as advancement if it ever does (e.g.
        // a property glitch during a VO reconfigure) -- strictly `>`, not
        // `!=`.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(5.0));
        m.tick(Some(4.0)); // stall #1 (went backwards)
        assert_eq!(
            m.tick(Some(4.0)), // stall #2
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }
}
