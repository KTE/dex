//! F10 — systemd `WatchdogSec=` + a hand-rolled `sd_notify`: the actor of
//! last resort for the one hazard F1/F9 cannot see. See PLAN.md's F10 entry
//! for the full design record (the "F9/F10 distinction" framing, the socket
//! analysis, the dependency decision, the timing math); this doc condenses
//! the parts an implementer reading this file needs, plus the parts a
//! reviewer of *this file specifically* needs.
//!
//! # Framing: what F10 is for, and why it is a separate feature from F1/F9
//!
//! F9 proved a wedged mpv core produces SILENCE on this program's supervisor
//! thread, not a block (`mpv_observe_property` getters run on the core
//! thread with the client lock dropped — verified against mpv 0.40 source,
//! see `dexd::health`'s module doc). F1 acts on that silence in-process,
//! with a bounded budget. Neither covers the one thing left: **our own supervisor
//! thread hanging in code that is not an mpv call at all** — the canonical
//! case being `eprintln!` blocking against a wedged journald, including on
//! the `Escalate` arm whose entire job is "exit so tier 1 can take over" (see
//! `main.rs::act_on_health_action`'s `Escalate` arm). Nothing in-process can
//! detect that; it needs an external actor. That is systemd, driven by
//! `WatchdogSec=` in the unit (`deploy/dexd.service`) plus this module.
//!
//! # The liveness criterion — and why a WEDGED PLAYER FAILS it
//!
//! `main.rs` pings `WATCHDOG=1` once per `HEALTH_CHECK_SECS` tick, **after**
//! that tick's `HealthMonitor::tick` evaluation (and any resulting recovery
//! command) has completed — never from anywhere else, and specifically never
//! from the 600s heartbeat (longer than any sane `WatchdogSec`). "The process
//! is alive" alone would be worthless (PLAN.md's own words) — the reason
//! this criterion is NOT that is a property F1 already guarantees: its
//! recovery budget (`MAX_RECOVERY_ATTEMPTS`, `main.rs`) is cumulative and
//! NEVER refills (see `dexd::health`'s "why the budget never resets"),
//! so a display-wedged player's tick sequence is forced, by construction,
//! through: silence → stall detected at 2 ticks (~20s) → ≤3 budgeted
//! recoveries (~20s each) → `Escalate` → `std::process::exit(1)`. A wedged
//! display therefore emits a BOUNDED number of pings and then either exits
//! (moot — tier 1's exit-code path already handles it) or, on the one path
//! that can still hang (`Escalate`'s own `eprintln!` before `exit(1)`, or a
//! hang anywhere else in the loop body), STOPS pinging entirely — because
//! the ping sits at the very end of a duty cycle that a genuine hang, by
//! definition, never completes again. Within this hazard class — a wedged
//! mpv core or a hung supervisor thread — there is no third state: either
//! progress continues (pings continue, correctly, nothing to do), or the
//! duty cycle stops completing (pings stop, watchdog fires). The loop
//! cannot both hang and ping.
//!
//! Scope, stated precisely: that guarantee covers wedged-core/wedged-thread
//! failures ONLY. A player whose `time-pos` keeps advancing while no photons
//! reach the wall — HDMI signal lost mid-run, panel powered off, the plane
//! presenting to a disconnected sink — reads as healthy to F1 (decode and
//! present proceed internally) and therefore pings forever. No in-process
//! liveness criterion can see that; it is a signal-level failure, explicitly
//! out of F10's scope — see PLAN.md's F10 entry ("residual states") for the
//! record and a possible future closure (DRM connector-status polling).
//!
//! # Gate placement: the ping is emitted OUTSIDE the health `Option` gate
//!
//! `main.rs` pings regardless of whether `health: Option<HealthMonitor>` is
//! `Some` — i.e. even in the near-zero-probability case where the `time-pos`
//! `mpv_observe_property` registration itself failed and F1 is DISABLED for
//! the run. In that mode the ping only certifies "the event loop completed
//! an iteration," a strictly weaker claim — but stopping pings there instead
//! would convert a degraded-but-otherwise-fine run into a guaranteed
//! `WatchdogSec`-later kill loop, which is a worse outcome than the
//! diagnostically-honest weaker claim. The DISABLED warning at startup gains
//! a clause saying so. See `main.rs`'s tick site for where this is wired.
//!
//! # No new blocking call — the socket analysis
//!
//! `sd_notify` is `sendto(2)` on an `AF_UNIX SOCK_DGRAM` socket to
//! `$NOTIFY_SOCKET`. Unlike UDP, Unix datagram sockets have flow control: if
//! the receiver's (PID 1's) queue is full, a *blocking* `sendto` blocks
//! rather than dropping. PID 1's queue is generally enormous relative to a
//! 10-byte payload, but "generally" was exactly the standard of proof F9's
//! lesson raised the bar past — so this module does not rely on it: the
//! socket this crate opens is put into **non-blocking mode**
//! (`UnixDatagram::set_nonblocking(true)`, done once in `main.rs`, the
//! socket's owner), and `EAGAIN`/`EWOULDBLOCK` is treated as a **dropped
//! ping**, never retried synchronously, never panicked on — see
//! [`interpret_send_result`] and [`send_ping`]. With a 10s ping cadence and
//! `WatchdogSec=180` (`deploy/dexd.service`), systemd needs ~17
//! consecutive drops before a spurious kill; a run that sick is not a wrong
//! restart. The dropped-ping count is surfaced in the 600s heartbeat line
//! (`watchdog=armed pings-dropped=N`, see `heartbeat.rs`) so a run degraded
//! this way is diagnosable after the fact, per this crate's "loud, never
//! silent" principle.
//!
//! # Dependency: zero. Hand-written, `std` only
//!
//! The entire protocol this crate needs is: read `$NOTIFY_SOCKET`, open an
//! unbound `AF_UNIX SOCK_DGRAM` socket, send the literal bytes `WATCHDOG=1`.
//! `std::os::unix::net::UnixDatagram` covers all of it, including the
//! abstract-namespace case (`std::os::linux::net::SocketAddrExt`, stable
//! since 1.70 — comfortably inside this crate's `rust-version = "1.85"`).
//! No `libsystemd` binding (would add a shared-object link, growing the
//! `.deb`'s `$auto`-derived `Depends`, to avoid ~150 lines) and no
//! `sd-notify` crate (removes less than it appears to — the ping policy,
//! the env handshake, and an audit of its socket handling for the
//! non-blocking guarantee this feature requires would all still be owned
//! here). See PLAN.md's F10 entry, §3, for the full comparison this crate's
//! SPEC §5c ("a dependency earns its place by removing code we would
//! otherwise own") is weighed against. `Cargo.toml`'s dependency list and
//! `$auto`-derived `.deb` `Depends` are both UNCHANGED by this feature.
//!
//! Deliberately **not** sent: `READY=1` (a no-op under `Type=simple` +
//! `NotifyAccess=main`, and sending it would invite a future switch to
//! `Type=notify` that PLAN.md explicitly forbids — a notify unit that never
//! sends `READY=1` sits inactive forever, and this program has no natural
//! "ready" moment before the endless stream starts). Also not sent:
//! `STOPPING=1` (no graceful shutdown exists, by design — the mains switch
//! IS the shutdown path).
//!
//! # A deliberate deviation from PLAN.md's literal test recipe, and why
//!
//! PLAN.md's §6 test plan calls for the non-blocking-guarantee test to
//! `shrink [the receiver's] SO_RCVBUF` via `setsockopt` before flooding it.
//! `std` exposes no safe way to do that, and this crate is
//! `#![forbid(unsafe_code)]` crate-wide (`lib.rs`) — a `forbid`, not a
//! `deny`, so it cannot be locally overridden even inside a `#[cfg(test)]`
//! module; that is the entire point of choosing `forbid` there. Rather than
//! reach for a `libc`/raw-FFI dependency (itself a SPEC §5c question this
//! feature's whole point is to avoid needing) just to shrink a buffer for a
//! test, [`tests::send_ping_eventually_reports_dropped_once_the_receiver_queue_fills`]
//! instead floods the OS's UNMODIFIED default receive buffer within a
//! bounded iteration count. A 10-byte datagram still costs real per-message
//! kernel accounting overhead, so even a generous default buffer fills
//! within at most a few thousand sends — the bound used has two orders of
//! magnitude of headroom over that. This is a real behavioural test (an
//! actual unread socket, actually exhausted) of the exact property this
//! module depends on, just without artificially engineering the buffer size
//! first. Flagged here per this crate's practice of stating a deviation
//! rather than silently reinterpreting the brief.

use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};

/// The entire wire payload this program ever sends. No trailing newline:
/// sd_notify's protocol is newline-separated `KEY=VALUE` lines, and this is
/// the only line this crate ever emits (see the module doc for why
/// `READY=1`/`STOPPING=1` are deliberately never sent) — a trailing newline
/// would be decoration systemd does not require (`sd-daemon` sources treat a
/// single-line unterminated datagram as a complete, valid notification).
pub const WATCHDOG_PING_PAYLOAD: &[u8] = b"WATCHDOG=1";

/// Raw environment inputs the handshake decision needs, as explicit values
/// rather than three direct `std::env::var` calls inside [`resolve`] — that
/// keeps `resolve` pure (no I/O, no global state) and testable with
/// synthetic inputs, without mutating the real process environment (which
/// races every other test in the same test binary; see `std::env::set_var`'s
/// own safety caveats). [`WatchdogEnv::from_process_env`] is the one place
/// that actually reads the real environment, for `main.rs` to call once at
/// startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchdogEnv {
    pub notify_socket: Option<String>,
    pub watchdog_pid: Option<String>,
    pub watchdog_usec: Option<String>,
}

impl WatchdogEnv {
    /// Read the three systemd notify-protocol variables from the real
    /// process environment. The only non-pure function in this module;
    /// everything downstream of it ([`resolve`], [`socket_addr`],
    /// [`send_ping`]) takes its inputs explicitly instead.
    pub fn from_process_env() -> Self {
        Self {
            notify_socket: std::env::var("NOTIFY_SOCKET").ok(),
            watchdog_pid: std::env::var("WATCHDOG_PID").ok(),
            watchdog_usec: std::env::var("WATCHDOG_USEC").ok(),
        }
    }
}

/// Where to send pings, in a form that survives being decided on ANY
/// platform. Kept as this crate's own enum rather than a real
/// `std::os::unix::net::SocketAddr` — turning `Abstract` into a concrete
/// address needs `std::os::linux::net::SocketAddrExt`, which does not exist
/// outside Linux/Android, so [`resolve`] (which must stay compilable and
/// testable on macOS, the dev machine) cannot construct one directly. See
/// [`socket_addr`] for the platform-gated conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifySocketAddr {
    /// A filesystem path — the common case (containers, VMs, most systemd
    /// configurations).
    Path(String),
    /// `$NOTIFY_SOCKET` began with `@`: an abstract-namespace name (the
    /// leading `@` already stripped, matching `sd_notify`'s own convention —
    /// see systemd's `sd-daemon.c`).
    Abstract(String),
}

/// Why the watchdog is inert for this run. Every arm here describes a
/// NORMAL run, not an error: the Mac dev machine, a bench tmux session, CI,
/// and any manual invocation off systemd all land in
/// [`InertReason::NoNotifySocket`] — the overwhelmingly common case. Logged
/// once at startup, not warned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InertReason {
    /// No `$NOTIFY_SOCKET` at all: not running under a systemd unit with
    /// `NotifyAccess=` set.
    NoNotifySocket,
    /// `$WATCHDOG_PID` is set and does not identify this process. The
    /// watchdog handshake belongs to a DIFFERENT process (e.g. a wrapper
    /// script systemd also tracks under the same unit) — pinging under
    /// someone else's identity would be actively wrong, not merely useless,
    /// so this is inert rather than armed-anyway. A value that fails to
    /// parse as a pid at all is treated identically to "set and different":
    /// it cannot possibly equal our own pid either way.
    WatchdogPidMismatch { ours: u32, unit: String },
}

impl std::fmt::Display for InertReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InertReason::NoNotifySocket => write!(f, "no NOTIFY_SOCKET"),
            InertReason::WatchdogPidMismatch { ours, unit } => write!(
                f,
                "WATCHDOG_PID={unit} does not match our pid {ours} -- the watchdog handshake \
                 belongs to a different process"
            ),
        }
    }
}

/// A startup condition worth a loud warning even though the watchdog IS (or
/// becomes) armed — distinct from [`InertReason`], which is never a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmedWarning {
    /// `$WATCHDOG_USEC` is set but small enough that this program's fixed
    /// ping cadence does not clear systemd's own "ping at least twice per
    /// window" guidance with any margin (PLAN.md F10 §4). Pings are still
    /// sent every tick regardless — there is nothing better to do from
    /// inside the process — this is purely diagnostic, flagging a likely
    /// unit misconfiguration rather than something this code can fix.
    WindowTooShortForCadence { window_secs: u64, tick_secs: u64 },
}

impl std::fmt::Display for ArmedWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArmedWarning::WindowTooShortForCadence { window_secs, tick_secs } => write!(
                f,
                "WatchdogSec window ({window_secs}s) gives our {tick_secs}s ping cadence little \
                 margin (systemd recommends pinging at least twice per window) -- likely a \
                 misconfigured unit; pings are still sent every tick regardless"
            ),
        }
    }
}

/// The one decision this module exists to make, taken once at startup from
/// [`WatchdogEnv`] alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchdogDecision {
    Inert(InertReason),
    Armed {
        addr: NotifySocketAddr,
        /// `$WATCHDOG_USEC`, converted to whole seconds, when systemd
        /// exported it (it does whenever `WatchdogSec=` is configured on
        /// the unit — absent only if `NotifyAccess=` was granted without
        /// `WatchdogSec=`, an unusual configuration). Carried through so
        /// `main.rs`'s startup log line can state the real window rather
        /// than the value this crate merely hopes the unit file has.
        window_secs: Option<u64>,
        warning: Option<ArmedWarning>,
    },
}

/// Decide whether — and where — to ping, from raw env strings alone. Pure:
/// no I/O, no socket, no env access (see [`WatchdogEnv::from_process_env`]
/// for the one caller that supplies real values). `tick_secs` is
/// `HEALTH_CHECK_SECS` (`main.rs`), passed in explicitly rather than
/// imported, so this module has zero dependency on `main`'s constants.
pub fn resolve(env: &WatchdogEnv, our_pid: u32, tick_secs: u64) -> WatchdogDecision {
    let Some(sock) = env.notify_socket.as_deref().filter(|s| !s.is_empty()) else {
        return WatchdogDecision::Inert(InertReason::NoNotifySocket);
    };

    if let Some(unit_pid) = env.watchdog_pid.as_deref() {
        let matches = unit_pid.parse::<u32>().map(|p| p == our_pid).unwrap_or(false);
        if !matches {
            return WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                ours: our_pid,
                unit: unit_pid.to_string(),
            });
        }
    }

    let addr = if let Some(name) = sock.strip_prefix('@') {
        NotifySocketAddr::Abstract(name.to_string())
    } else {
        NotifySocketAddr::Path(sock.to_string())
    };

    // A malformed WATCHDOG_USEC (never expected from a real systemd, but
    // this is text from the environment, not a value this program controls)
    // fails safe: no window is known, so no "too short" warning is invented
    // either -- see the "window unknown" rendering this leaves to main.rs.
    let window_secs = env
        .watchdog_usec
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|usec| usec / 1_000_000);

    let warning = window_secs.and_then(|w| {
        (w < tick_secs.saturating_mul(2))
            .then_some(ArmedWarning::WindowTooShortForCadence { window_secs: w, tick_secs })
    });

    WatchdogDecision::Armed { addr, window_secs, warning }
}

/// Resolve a [`NotifySocketAddr`] into the concrete address `send_to_addr`
/// needs. `Err` only for [`NotifySocketAddr::Abstract`] on a non-Linux
/// build (`std::os::linux::net::SocketAddrExt` does not exist there) --
/// unreachable in the one place that matters (a real systemd host is
/// Linux), and exists only so this module still compiles and is readable on
/// the Mac dev machine rather than `#[cfg]`-ing the whole abstract-name
/// branch out of existence there.
pub fn socket_addr(target: &NotifySocketAddr) -> io::Result<SocketAddr> {
    match target {
        NotifySocketAddr::Path(p) => SocketAddr::from_pathname(p),
        NotifySocketAddr::Abstract(name) => abstract_addr(name),
    }
}

#[cfg(target_os = "linux")]
fn abstract_addr(name: &str) -> io::Result<SocketAddr> {
    use std::os::linux::net::SocketAddrExt;
    SocketAddr::from_abstract_name(name.as_bytes())
}

#[cfg(not(target_os = "linux"))]
fn abstract_addr(_name: &str) -> io::Result<SocketAddr> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "abstract-namespace notify sockets require Linux (std::os::linux::net::SocketAddrExt) -- \
         a $NOTIFY_SOCKET starting with '@' can only be set by a real systemd host",
    ))
}

/// Outcome of one ping attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingOutcome {
    Sent,
    /// The datagram did not go out -- EAGAIN/EWOULDBLOCK (receiver's queue
    /// full) or any other send error (e.g. the notify socket path vanishing
    /// mid-run). Every failure mode is treated identically as "try again
    /// next tick": see the module doc's socket analysis for why this is
    /// never escalated, never retried synchronously, and never panics.
    Dropped,
}

/// The ONLY place that decides "was this ping delivered" -- separated from
/// the I/O call itself so the policy (a failed send degrades, it is never
/// fatal and never retried inline) is directly testable with synthetic
/// [`io::Result`]s, without a real socket at all.
pub fn interpret_send_result(result: io::Result<usize>) -> PingOutcome {
    match result {
        Ok(_) => PingOutcome::Sent,
        Err(_) => PingOutcome::Dropped,
    }
}

/// Send one watchdog ping over a socket the caller has already opened
/// non-blocking (`main.rs` owns creating and holding that socket across the
/// process's whole life, matching every other piece of process-lifetime
/// state in this crate's driver -- this function itself performs no setup,
/// so it cannot silently forget to set non-blocking mode). Never blocks,
/// never panics on a send failure: see [`interpret_send_result`].
pub fn send_ping(socket: &UnixDatagram, addr: &SocketAddr) -> PingOutcome {
    interpret_send_result(socket.send_to_addr(WATCHDOG_PING_PAYLOAD, addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(notify_socket: Option<&str>, watchdog_pid: Option<&str>, watchdog_usec: Option<&str>) -> WatchdogEnv {
        WatchdogEnv {
            notify_socket: notify_socket.map(str::to_string),
            watchdog_pid: watchdog_pid.map(str::to_string),
            watchdog_usec: watchdog_usec.map(str::to_string),
        }
    }

    // ---- resolve(): the handshake decision ----------------------------

    #[test]
    fn no_notify_socket_is_inert() {
        let d = resolve(&env(None, None, None), 1234, 10);
        assert_eq!(d, WatchdogDecision::Inert(InertReason::NoNotifySocket));
    }

    #[test]
    fn empty_notify_socket_is_treated_as_absent() {
        // Defensive: an exported-but-empty value is not a real address
        // either way, and must not be handed to socket construction as one.
        let d = resolve(&env(Some(""), None, None), 1234, 10);
        assert_eq!(d, WatchdogDecision::Inert(InertReason::NoNotifySocket));
    }

    #[test]
    fn path_socket_with_no_pid_or_usec_arms_with_no_window_or_warning() {
        let d = resolve(&env(Some("/run/systemd/notify"), None, None), 1234, 10);
        assert_eq!(
            d,
            WatchdogDecision::Armed {
                addr: NotifySocketAddr::Path("/run/systemd/notify".to_string()),
                window_secs: None,
                warning: None,
            }
        );
    }

    #[test]
    fn at_prefixed_socket_resolves_to_an_abstract_address_with_the_prefix_stripped() {
        let d = resolve(&env(Some("@abstract-name"), None, None), 1234, 10);
        match d {
            WatchdogDecision::Armed { addr: NotifySocketAddr::Abstract(name), .. } => {
                assert_eq!(name, "abstract-name");
            }
            other => panic!("expected Armed/Abstract, got {other:?}"),
        }
    }

    #[test]
    fn matching_watchdog_pid_still_arms() {
        let d = resolve(&env(Some("/run/systemd/notify"), Some("1234"), None), 1234, 10);
        assert!(matches!(d, WatchdogDecision::Armed { .. }), "{d:?}");
    }

    #[test]
    fn mismatched_watchdog_pid_is_inert() {
        let d = resolve(&env(Some("/run/systemd/notify"), Some("999"), None), 1234, 10);
        assert_eq!(
            d,
            WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                ours: 1234,
                unit: "999".to_string(),
            })
        );
    }

    #[test]
    fn unparseable_watchdog_pid_is_treated_as_mismatched_not_as_absent() {
        let d = resolve(&env(Some("/run/systemd/notify"), Some("not-a-pid"), None), 1234, 10);
        assert_eq!(
            d,
            WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                ours: 1234,
                unit: "not-a-pid".to_string(),
            })
        );
    }

    #[test]
    fn watchdog_usec_with_ample_margin_arms_with_no_warning() {
        // 180s window (systemd's exported microseconds), 10s cadence: 18
        // pings per window, comfortably over the 2x guidance.
        let d = resolve(&env(Some("/run/systemd/notify"), None, Some("180000000")), 1234, 10);
        match d {
            WatchdogDecision::Armed { window_secs, warning, .. } => {
                assert_eq!(window_secs, Some(180));
                assert_eq!(warning, None);
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn watchdog_usec_below_twice_the_cadence_warns() {
        // 15s window, 10s cadence: only 1.5 pings per window, under
        // systemd's own "at least twice" guidance.
        let d = resolve(&env(Some("/run/systemd/notify"), None, Some("15000000")), 1234, 10);
        match d {
            WatchdogDecision::Armed { window_secs, warning, .. } => {
                assert_eq!(window_secs, Some(15));
                assert_eq!(
                    warning,
                    Some(ArmedWarning::WindowTooShortForCadence { window_secs: 15, tick_secs: 10 })
                );
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn watchdog_usec_exactly_twice_the_cadence_does_not_warn() {
        // The boundary: 20s window / 10s cadence is exactly systemd's own
        // "at least twice" guidance, not yet "less than" it.
        let d = resolve(&env(Some("/run/systemd/notify"), None, Some("20000000")), 1234, 10);
        match d {
            WatchdogDecision::Armed { warning, .. } => assert_eq!(warning, None),
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_watchdog_usec_arms_with_no_window_and_no_invented_warning() {
        let d = resolve(&env(Some("/run/systemd/notify"), None, Some("soon")), 1234, 10);
        match d {
            WatchdogDecision::Armed { window_secs, warning, .. } => {
                assert_eq!(window_secs, None);
                assert_eq!(warning, None);
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    // ---- the wire payload ----------------------------------------------

    #[test]
    fn ping_payload_is_exactly_watchdog_1_no_trailing_newline() {
        assert_eq!(WATCHDOG_PING_PAYLOAD, b"WATCHDOG=1");
    }

    // ---- interpret_send_result: pure Ok/Err -> outcome mapping --------

    #[test]
    fn a_successful_send_is_sent() {
        assert_eq!(interpret_send_result(Ok(10)), PingOutcome::Sent);
    }

    #[test]
    fn would_block_is_dropped_not_an_error() {
        let err = io::Error::from(io::ErrorKind::WouldBlock);
        assert_eq!(interpret_send_result(Err(err)), PingOutcome::Dropped);
    }

    #[test]
    fn any_other_send_failure_also_degrades_to_dropped_never_panics() {
        // e.g. the notify socket path vanishing mid-run (ENOENT) -- see the
        // module doc: every failure mode is "try again next tick", none is
        // special-cased or escalated.
        let err = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(interpret_send_result(Err(err)), PingOutcome::Dropped);
    }

    // ---- socket_addr(): platform-gated address construction -----------

    #[test]
    fn socket_addr_resolves_a_path_target() {
        let target = NotifySocketAddr::Path("/tmp/dexd-watchdog-example.sock".to_string());
        let addr = socket_addr(&target).expect("path address resolves");
        assert_eq!(
            addr.as_pathname(),
            Some(std::path::Path::new("/tmp/dexd-watchdog-example.sock"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn socket_addr_resolves_an_abstract_target_on_linux() {
        let target = NotifySocketAddr::Abstract("dexd-watchdog-test".to_string());
        let addr = socket_addr(&target);
        assert!(addr.is_ok(), "{addr:?}");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn socket_addr_refuses_an_abstract_target_off_linux() {
        let target = NotifySocketAddr::Abstract("dexd-watchdog-test".to_string());
        let err = socket_addr(&target).expect_err("abstract names require Linux");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    // ---- send_ping(): real, unmocked sockets ---------------------------

    /// `/tmp` directly, NOT `std::env::temp_dir()`: `AF_UNIX` paths are
    /// capped at `sizeof(sockaddr_un.sun_path)` (104 bytes on macOS, 108 on
    /// Linux), and macOS's real temp dir
    /// (`/var/folders/.../T/`) is already close to that budget on its own --
    /// `std::env::temp_dir()` here produced `EINVAL: path must be shorter
    /// than SUN_LEN` in practice. A short, fixed prefix plus a process-local
    /// atomic counter keeps every path well under the limit on both
    /// platforms without needing to reason about `$TMPDIR`'s length.
    fn unique_socket_path(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::path::PathBuf::from(format!("/tmp/dwd-{:x}-{n:x}-{label}.sock", std::process::id()))
    }

    #[test]
    fn send_ping_delivers_the_exact_payload_to_a_real_receiver() {
        let path = unique_socket_path("happy");
        let _ = std::fs::remove_file(&path);
        let receiver = UnixDatagram::bind(&path).expect("bind receiver");
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("set_read_timeout");

        let addr = SocketAddr::from_pathname(&path).expect("path address");
        let sender = UnixDatagram::unbound().expect("unbound sender");
        sender.set_nonblocking(true).expect("set_nonblocking");

        assert_eq!(send_ping(&sender, &addr), PingOutcome::Sent);

        let mut buf = [0u8; 32];
        let n = receiver.recv(&mut buf).expect("recv");
        assert_eq!(&buf[..n], WATCHDOG_PING_PAYLOAD);

        drop(receiver);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn send_ping_eventually_reports_dropped_once_the_receiver_queue_fills() {
        // See the module doc's "deliberate deviation" section for why this
        // floods the OS's UNMODIFIED default receive buffer, bounded by
        // ATTEMPTS, rather than shrinking SO_RCVBUF first.
        let path = unique_socket_path("flood");
        let _ = std::fs::remove_file(&path);
        let receiver = UnixDatagram::bind(&path).expect("bind receiver");
        // Deliberately never read from `receiver`.

        let addr = SocketAddr::from_pathname(&path).expect("path address");
        let sender = UnixDatagram::unbound().expect("unbound sender");
        sender.set_nonblocking(true).expect("set_nonblocking");

        const ATTEMPTS: usize = 200_000;
        let mut saw_dropped = false;
        for _ in 0..ATTEMPTS {
            if send_ping(&sender, &addr) == PingOutcome::Dropped {
                saw_dropped = true;
                break;
            }
        }

        drop(receiver);
        let _ = std::fs::remove_file(&path);

        assert!(
            saw_dropped,
            "expected send_to_addr to eventually return WouldBlock (-> Dropped) against an \
             unread receiver within {ATTEMPTS} sends -- if this fails, either this platform's \
             default SO_RCVBUF is unexpectedly huge, or the non-blocking guarantee broke \
             (set_nonblocking silently not honoured would hang this loop instead of failing \
             this assertion -- see the module doc)"
        );
    }
}
