//! Stall detection and the in-place recovery policy for the periodic health
//! check, as plain state machines with no libmpv, display or FFI behind them,
//! so the policy can be tested on its own. `main.rs` samples mpv's playback
//! position on a fixed cadence, feeds each sample to [`HealthMonitor::tick`]
//! and carries out the [`HealthAction`] that comes back.
//!
//! The position reaches this module only through the property-change events
//! mpv pushes, never through a synchronous property read, so judging health
//! cannot itself block the supervisor thread inside a stalled mpv core.
//!
//! See docs/design/failure-handling.md#health-check for the policy, the
//! recovery budget and what an in-place recovery does and does not rebuild.

/// What the caller should do after feeding one health-check sample to
/// [`HealthMonitor::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAction {
    /// The position advanced, or there is not enough history to judge yet:
    /// the first sample, or a single non-advancing one. Nothing to do.
    Healthy,
    /// The position has not advanced across two consecutive samples and the
    /// recovery budget still has attempts left. The caller re-issues
    /// `loadfile ... replace` asynchronously and keeps running.
    AttemptRecovery {
        /// This attempt's number, counting from 1, against `max`.
        attempt: u32,
        max: u32,
    },
    /// The position has not advanced across two consecutive samples and the
    /// recovery budget is spent. The caller exits non-zero so systemd
    /// restarts the process.
    Escalate,
}

/// The stall-detection state machine: two consecutive non-advancing samples
/// count as a stall, and every recovery it grants is drawn from one budget
/// that spans the life of the process.
///
/// See docs/design/failure-handling.md#health-check.
pub struct HealthMonitor {
    last_position: Option<f64>,
    consecutive_stalls: u32,
    recovery_attempts_used: u32,
    max_recovery_attempts: u32,
}

impl HealthMonitor {
    /// `max_recovery_attempts` is the whole budget for the life of the
    /// process. It never refills, so a fault that heals and stalls again
    /// cannot earn further attempts.
    ///
    /// See docs/design/failure-handling.md#recovery-budget.
    pub fn new(max_recovery_attempts: u32) -> Self {
        Self {
            last_position: None,
            consecutive_stalls: 0,
            recovery_attempts_used: 0,
            max_recovery_attempts,
        }
    }

    /// Feed one health-check sample. `position` is the latest known playback
    /// position in seconds, or `None` when no update has arrived since the
    /// monitor was created or since a recovery cleared its baseline. A stalled
    /// mpv core stops sending updates at all, so `None` is judged the same as
    /// an unchanged value.
    ///
    /// See docs/design/failure-handling.md#health-check.
    pub fn tick(&mut self, position: Option<f64>) -> HealthAction {
        let advanced = match (position, self.last_position) {
            (Some(p), Some(prev)) => p > prev,
            // First sample since start, or since a recovery cleared the
            // baseline below: there is nothing to compare against. Counting it
            // as progress keeps mpv's own startup and reload latency, a few
            // hundred milliseconds to a few seconds before a 3840x2160 decode
            // begins, from reading as a fault. A startup that produces no
            // sample at all is still caught: that is two `None` ticks below.
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

        // Two consecutive non-advancing samples: a stall. Clear the jitter
        // counter so the attempt that follows, or the escalation, gets its own
        // two-sample window instead of re-triggering on the next tick. The
        // budget is untouched here; only `issue_recovery_or_escalate` spends
        // it.
        self.consecutive_stalls = 0;
        self.issue_recovery_or_escalate()
    }

    /// Produce the recovery decision a qualifying stall would produce, without
    /// waiting for one (test rig only; `main.rs` arms it from
    /// `--test-rig-force-recovery-after-secs`). It spends the same budget and
    /// clears the same position baseline as [`HealthMonitor::tick`], so the
    /// caller drives the identical mpv code path.
    ///
    /// See docs/design/failure-handling.md#test-rig-probes.
    pub fn force_recovery(&mut self) -> HealthAction {
        self.consecutive_stalls = 0;
        self.issue_recovery_or_escalate()
    }

    /// The budget check and the attempt-or-escalate decision shared by
    /// [`HealthMonitor::tick`]'s stall arm and
    /// [`HealthMonitor::force_recovery`] — the only two places that spend the
    /// recovery budget. Each caller does its own stall bookkeeping first.
    fn issue_recovery_or_escalate(&mut self) -> HealthAction {
        if self.recovery_attempts_used >= self.max_recovery_attempts {
            return HealthAction::Escalate;
        }
        self.recovery_attempts_used += 1;
        // A recovery re-opens the stream from byte 0: `main.rs` issues
        // `loadfile ... replace` on the same loop:// URL, which creates a
        // fresh stream, so mpv's position counter restarts near zero. Drop the
        // baseline here, or the next real sample would compare below the
        // pre-recovery high-water mark and read as a continuing stall.
        self.last_position = None;
        HealthAction::AttemptRecovery {
            attempt: self.recovery_attempts_used,
            max: self.max_recovery_attempts,
        }
    }
}

/// Decides when to fire one forced recovery, independent of whether anything
/// has stalled (test rig only). It fires once per process, so a run exercises
/// the recovery path against a real mpv one time; `main.rs` arms it from
/// `--test-rig-force-recovery-after-secs` and
/// [`HealthMonitor::force_recovery`] is what firing calls.
///
/// See docs/design/failure-handling.md#test-rig-probes.
pub struct ForceRecoveryTrigger {
    after_secs: u64,
    fired: bool,
}

impl ForceRecoveryTrigger {
    pub fn new(after_secs: u64) -> Self {
        Self {
            after_secs,
            fired: false,
        }
    }

    /// Feed the current uptime. Returns `true` on the first call where
    /// `uptime_secs >= after_secs`, and `false` on every call before and
    /// after that one.
    pub fn should_fire(&mut self, uptime_secs: u64) -> bool {
        if self.fired || uptime_secs < self.after_secs {
            return false;
        }
        self.fired = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sample_is_healthy_with_no_position_to_compare() {
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(0.0)), HealthAction::Healthy);
    }

    #[test]
    fn a_steadily_advancing_position_never_triggers_an_action() {
        let mut m = HealthMonitor::new(3);
        let mut pos = 0.0;
        for _ in 0..50 {
            pos += 0.033; // roughly one frame at 30 fps per tick
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }
    }

    #[test]
    fn a_single_non_advancing_tick_is_not_enough_to_act() {
        // One stalled sample must trigger nothing, or ordinary jitter around a
        // check boundary would cause spurious recoveries.
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy); // stall 1
        assert_eq!(m.tick(Some(2.0)), HealthAction::Healthy); // advances again
    }

    #[test]
    fn two_consecutive_stalls_trigger_the_first_recovery_attempt() {
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy);
        assert_eq!(m.tick(Some(1.0)), HealthAction::Healthy); // stall 1
        assert_eq!(
            m.tick(Some(1.0)), // stall 2 — qualifies
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn a_missing_sample_counts_as_a_stall_like_an_unchanged_position() {
        // No property-change event arriving is the same symptom as a value
        // that does not change. A `Some` value gets one tick of grace to
        // absorb startup latency; `None` gets none, since a run of `None`s is
        // the startup that never produces a position at all.
        let mut m = HealthMonitor::new(3);
        assert_eq!(m.tick(None), HealthAction::Healthy); // stall 1
        assert_eq!(
            m.tick(None), // stall 2 — qualifies, no grace for None
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn recovery_gets_a_fresh_two_tick_window_before_being_judged_again() {
        let mut m = HealthMonitor::new(3);
        m.tick(Some(1.0));
        m.tick(Some(1.0)); // stall 1
        assert_eq!(
            m.tick(Some(1.0)), // stall 2: attempt 1
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // One further sample right after an attempt must not re-trigger: the
        // attempt needs its own two-tick window, or a recovery that has had no
        // time to take effect is judged as having already failed.
        assert_eq!(m.tick(Some(0.05)), HealthAction::Healthy);
    }

    #[test]
    fn recovery_clears_the_position_baseline_so_a_restarted_stream_reads_as_progress() {
        // A `loadfile ... replace` opens a fresh loop:// stream, so mpv's
        // position counter restarts near zero. Without clearing the monitor's
        // baseline, the next real sample would compare below the pre-recovery
        // high-water mark and register as a further stall.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(500.0));
        m.tick(Some(500.0)); // stall 1
        assert_eq!(
            m.tick(Some(500.0)), // stall 2: recovery issued, baseline dropped
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // The reloaded stream starts near zero, which must read as progress.
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
        // The stream advances again — the recovery worked — and stays healthy.
        let mut pos = 0.0;
        for _ in 0..20 {
            pos += 0.1;
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }
    }

    #[test]
    fn budget_is_cumulative_and_never_refills_across_separate_episodes() {
        // A fault that heals between episodes does not get its budget back. A
        // budget that refilled would let a flapping fault run an unbounded
        // total of in-place recoveries, spread across many episodes.
        let mut m = HealthMonitor::new(2);

        // Episode 1: stalls, recovers (attempt 1 of 2).
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(
            m.tick(Some(1.0)),
            HealthAction::AttemptRecovery { attempt: 1, max: 2 }
        );
        // Healthy for a long stretch afterwards.
        let mut pos = 0.0;
        for _ in 0..100 {
            pos += 0.1;
            assert_eq!(m.tick(Some(pos)), HealthAction::Healthy);
        }

        // Episode 2: stalls again, recovers (attempt 2 of 2 — budget spent).
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

        // Episode 3: the budget is gone, so this stall escalates instead of
        // taking a third recovery, however healthy the hundreds of ticks in
        // between were.
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
        // The budget is spent (1 of 1 used), so the next qualifying stall
        // escalates instead of taking a second recovery.
        m.tick(Some(0.1));
        m.tick(Some(0.1));
        assert_eq!(m.tick(Some(0.1)), HealthAction::Escalate);
    }

    #[test]
    fn zero_budget_escalates_on_the_first_qualifying_stall() {
        // A monitor built with no recovery budget still needs two consecutive
        // stalls — the jitter guard is unconditional — and then escalates
        // without ever attempting an in-place recovery.
        let mut m = HealthMonitor::new(0);
        m.tick(Some(1.0));
        m.tick(Some(1.0));
        assert_eq!(m.tick(Some(1.0)), HealthAction::Escalate);
    }

    #[test]
    fn position_going_backwards_counts_as_a_stall() {
        // A clock-driven playback position only moves forward, but if a
        // property glitch during a video-output reconfigure ever moves it
        // back, that counts as a stall: the comparison is `>`, not `!=`.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(5.0));
        m.tick(Some(4.0)); // stall 1 (went backwards)
        assert_eq!(
            m.tick(Some(4.0)), // stall 2
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    // ---- Forced recovery, test rig only --------------------------------

    #[test]
    fn force_recovery_attempts_immediately_with_no_stall_observed() {
        // Unlike `tick`, this needs no stall history: steadily advancing
        // playback is still forced into a recovery attempt.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(1.0));
        m.tick(Some(2.0));
        m.tick(Some(3.0)); // steady progress, nowhere near a stall
        assert_eq!(
            m.force_recovery(),
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn force_recovery_shares_the_same_cumulative_budget_as_tick() {
        // A forced attempt spends from the budget an organic stall would
        // spend. Two separate counters would let the total number of in-place
        // recoveries exceed `max_recovery_attempts`.
        let mut m = HealthMonitor::new(2);
        assert_eq!(
            m.force_recovery(),
            HealthAction::AttemptRecovery { attempt: 1, max: 2 }
        );
        m.tick(Some(10.0));
        m.tick(Some(10.0)); // stall 1
        assert_eq!(
            m.tick(Some(10.0)), // stall 2 — the second and last budgeted attempt
            HealthAction::AttemptRecovery { attempt: 2, max: 2 }
        );
        // One forced plus one organic attempt spends the budget, so a third
        // request of either kind escalates.
        assert_eq!(m.force_recovery(), HealthAction::Escalate);
    }

    #[test]
    fn force_recovery_escalates_once_the_budget_is_already_exhausted() {
        let mut m = HealthMonitor::new(0);
        assert_eq!(m.force_recovery(), HealthAction::Escalate);
    }

    #[test]
    fn force_recovery_clears_the_position_baseline_like_a_stall_recovery() {
        // The forced recovery's `loadfile ... replace` restarts mpv's position
        // counter near zero, so the monitor's baseline drops with it and the
        // next real sample reads as progress.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(500.0));
        assert_eq!(
            m.force_recovery(),
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        assert_eq!(m.tick(Some(0.1)), HealthAction::Healthy);
    }

    #[test]
    fn force_recovery_clears_accumulated_jitter_so_the_next_stall_needs_its_own_two_ticks() {
        // One non-advancing tick, which does not yet qualify, followed by a
        // forced recovery: the lone stall is not banked, so the next judgment
        // needs its own two-tick window.
        let mut m = HealthMonitor::new(3);
        m.tick(Some(1.0));
        m.tick(Some(1.0)); // stall 1 — does not yet qualify
        assert_eq!(
            m.force_recovery(),
            HealthAction::AttemptRecovery { attempt: 1, max: 3 }
        );
        // Had the earlier stall survived, this single non-advancing tick would
        // qualify as stall 2. It must not.
        assert_eq!(m.tick(Some(0.1)), HealthAction::Healthy);
    }

    #[test]
    fn force_recovery_trigger_does_not_fire_before_its_deadline() {
        let mut t = ForceRecoveryTrigger::new(10);
        assert!(!t.should_fire(0));
        assert!(!t.should_fire(9));
    }

    #[test]
    fn force_recovery_trigger_fires_once_at_the_deadline() {
        let mut t = ForceRecoveryTrigger::new(10);
        assert!(!t.should_fire(9));
        assert!(t.should_fire(10));
        // A later poll at the same or a higher uptime must not fire again:
        // one forced recovery per run.
        assert!(!t.should_fire(10));
        assert!(!t.should_fire(11));
        assert!(!t.should_fire(1_000_000));
    }

    #[test]
    fn force_recovery_trigger_fires_late_if_polled_late_but_still_only_once() {
        // `main.rs` polls this on a cadence, so a poll that lands after the
        // deadline still fires, once, instead of waiting for an exact match.
        let mut t = ForceRecoveryTrigger::new(10);
        assert!(!t.should_fire(3));
        assert!(t.should_fire(47));
        assert!(!t.should_fire(48));
    }

    #[test]
    fn force_recovery_trigger_zero_secs_fires_on_the_first_poll() {
        let mut t = ForceRecoveryTrigger::new(0);
        assert!(t.should_fire(0));
        assert!(!t.should_fire(0));
    }
}
