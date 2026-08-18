//! The systemd watchdog handshake: decide whether to ping, and send one
//! `WATCHDOG=1` datagram per health-check tick.
//!
//! `main.rs` sends the ping at the end of each tick, after the health check
//! and any recovery that tick ordered, so a supervisor thread that hangs
//! anywhere in the tick stops pinging and systemd restarts the process. Do
//! not ping from any other path: a ping sent from the heartbeat or from an
//! exit path resets systemd's countdown over the hang this module exists to
//! catch.
//!
//! See docs/design/failure-handling.md#systemd-watchdog for what a ping
//! certifies, docs/design/failure-handling.md#ping-protocol for the wire
//! protocol and the startup handshake, and docs/design/packaging.md#crates
//! for why the protocol is written against `std` alone.

use std::io;
use std::os::unix::net::{SocketAddr, UnixDatagram};

/// The one message dexd sends over the notify socket. No trailing newline:
/// the protocol separates `KEY=VALUE` lines with newlines, and a single
/// unterminated line is a complete notification.
pub const WATCHDOG_PING_PAYLOAD: &[u8] = b"WATCHDOG=1";

/// The three systemd notify-protocol variables, read once and handed to
/// [`resolve`] as values, so the decision does no I/O and no test has to
/// change the process environment.
/// See docs/design/failure-handling.md#ping-protocol.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchdogEnv {
    pub notify_socket: Option<String>,
    pub watchdog_pid: Option<String>,
    pub watchdog_usec: Option<String>,
}

impl WatchdogEnv {
    /// Read the three variables from the process environment. The only
    /// function here that touches global state; [`resolve`], [`socket_addr`]
    /// and [`send_ping`] take their inputs explicitly.
    pub fn from_process_env() -> Self {
        Self {
            notify_socket: std::env::var("NOTIFY_SOCKET").ok(),
            watchdog_pid: std::env::var("WATCHDOG_PID").ok(),
            watchdog_usec: std::env::var("WATCHDOG_USEC").ok(),
        }
    }
}

/// Where to send pings, as this module's own enum because an abstract name
/// needs `std::os::linux::net::SocketAddrExt`, which exists only on Linux,
/// while [`resolve`] also builds and runs on macOS. [`socket_addr`] does the
/// platform-gated conversion.
/// See docs/design/development.md#portability-details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifySocketAddr {
    /// A filesystem path — the common case (containers, virtual machines,
    /// most systemd configurations).
    Path(String),
    /// `$NOTIFY_SOCKET` began with `@`: an abstract-namespace name, with the
    /// leading `@` stripped as `sd_notify` strips it.
    Abstract(String),
}

/// Why no pings are sent for this run. Every arm describes a normal run: a
/// development machine, a test rig, CI and any invocation outside systemd all
/// land in [`InertReason::NoNotifySocket`]. Logged once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InertReason {
    /// No `$NOTIFY_SOCKET`: the process is not running under a systemd unit
    /// with `NotifyAccess=` set.
    NoNotifySocket,
    /// `$WATCHDOG_PID` is set and names another process — a wrapper script
    /// systemd also tracks under the same unit, for example — so the
    /// handshake belongs to that process and pinging under its identity
    /// would be wrong. A value that does not parse as a pid is treated the
    /// same way: it cannot equal this process's pid either.
    WatchdogPidMismatch { dexd_pid: u32, unit: String },
}

impl std::fmt::Display for InertReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InertReason::NoNotifySocket => write!(f, "no NOTIFY_SOCKET"),
            InertReason::WatchdogPidMismatch { dexd_pid, unit } => write!(
                f,
                "WATCHDOG_PID={unit} does not match this process's pid {dexd_pid} -- the watchdog \
                 handshake belongs to a different process"
            ),
        }
    }
}

/// A misconfigured unit, reported once at startup on a run that does send
/// pings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmedWarning {
    /// `$WATCHDOG_USEC` names a window shorter than twice the ping cadence,
    /// the margin systemd's documentation recommends. Pings still go out
    /// every tick; the unit is what needs correcting.
    WindowTooShortForCadence { window_secs: u64, tick_secs: u64 },
}

impl std::fmt::Display for ArmedWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArmedWarning::WindowTooShortForCadence {
                window_secs,
                tick_secs,
            } => write!(
                f,
                "WatchdogSec window ({window_secs}s) gives the {tick_secs}s ping cadence little \
                 margin (systemd recommends pinging at least twice per window) -- likely a \
                 misconfigured unit; pings are still sent every tick regardless"
            ),
        }
    }
}

/// The decision this module makes, taken once at startup from
/// [`WatchdogEnv`] alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchdogDecision {
    Inert(InertReason),
    Armed {
        addr: NotifySocketAddr,
        /// `$WATCHDOG_USEC` in whole seconds, when systemd exported it. It
        /// is absent only where `NotifyAccess=` was granted without
        /// `WatchdogSec=`. Carried through so the startup log line can name
        /// the window the unit set.
        window_secs: Option<u64>,
        warning: Option<ArmedWarning>,
    },
}

/// Decide whether, and where, to ping, from the environment strings alone.
/// No I/O, no socket and no environment access, so a test can drive every arm
/// with synthetic values; [`WatchdogEnv::from_process_env`] supplies the real
/// ones. `tick_secs` is the health-check interval, passed in so this module
/// depends on none of `main.rs`'s constants.
/// See docs/design/failure-handling.md#ping-protocol.
pub fn resolve(env: &WatchdogEnv, dexd_pid: u32, tick_secs: u64) -> WatchdogDecision {
    let Some(sock) = env.notify_socket.as_deref().filter(|s| !s.is_empty()) else {
        return WatchdogDecision::Inert(InertReason::NoNotifySocket);
    };

    if let Some(unit_pid) = env.watchdog_pid.as_deref() {
        let matches = unit_pid
            .parse::<u32>()
            .map(|p| p == dexd_pid)
            .unwrap_or(false);
        if !matches {
            return WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                dexd_pid,
                unit: unit_pid.to_string(),
            });
        }
    }

    let addr = if let Some(name) = sock.strip_prefix('@') {
        NotifySocketAddr::Abstract(name.to_string())
    } else {
        NotifySocketAddr::Path(sock.to_string())
    };

    // WATCHDOG_USEC is arbitrary text from the environment. A value that does
    // not parse leaves the window unknown, and no "window too short" warning
    // is derived from an unknown window; main.rs renders the unknown window in
    // the startup line.
    let window_secs = env
        .watchdog_usec
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|usec| usec / 1_000_000);

    let warning = window_secs.and_then(|w| {
        (w < tick_secs.saturating_mul(2)).then_some(ArmedWarning::WindowTooShortForCadence {
            window_secs: w,
            tick_secs,
        })
    });

    WatchdogDecision::Armed {
        addr,
        window_secs,
        warning,
    }
}

/// Turn a [`NotifySocketAddr`] into the concrete address `send_to_addr`
/// needs. `Err` only for [`NotifySocketAddr::Abstract`] off Linux, where
/// `std::os::linux::net::SocketAddrExt` does not exist; a systemd host is
/// always Linux, so that arm only keeps the module building on macOS.
/// See docs/design/development.md#portability-details.
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
    /// The datagram did not go out: the receiver's queue was full, or the
    /// send failed for another reason such as the notify socket path
    /// vanishing mid-run. Every failure means the same thing here, try again
    /// next tick, so none is escalated, retried inline or panicked on.
    /// See docs/design/failure-handling.md#ping-protocol.
    Dropped,
}

/// Map one send result to an outcome. Split from the I/O call so the policy —
/// every failure degrades to a dropped ping — is testable with synthetic
/// [`io::Result`]s and no socket.
pub fn interpret_send_result(result: io::Result<usize>) -> PingOutcome {
    match result {
        Ok(_) => PingOutcome::Sent,
        Err(_) => PingOutcome::Dropped,
    }
}

/// Send one watchdog ping over a socket the caller has already set
/// non-blocking. `main.rs` creates that socket once and holds it for the life
/// of the process; this function performs no setup. Keep the caller's
/// `set_nonblocking`: a blocking send parks the supervisor thread against a
/// full receiver queue, which is the fault the watchdog exists to catch. A
/// send failure returns [`PingOutcome::Dropped`] and never panics.
pub fn send_ping(socket: &UnixDatagram, addr: &SocketAddr) -> PingOutcome {
    interpret_send_result(socket.send_to_addr(WATCHDOG_PING_PAYLOAD, addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(
        notify_socket: Option<&str>,
        watchdog_pid: Option<&str>,
        watchdog_usec: Option<&str>,
    ) -> WatchdogEnv {
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
        // An exported but empty value is not an address, and must not reach
        // socket construction as one.
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
            WatchdogDecision::Armed {
                addr: NotifySocketAddr::Abstract(name),
                ..
            } => {
                assert_eq!(name, "abstract-name");
            }
            other => panic!("expected Armed/Abstract, got {other:?}"),
        }
    }

    #[test]
    fn a_watchdog_pid_matching_this_process_arms() {
        let d = resolve(
            &env(Some("/run/systemd/notify"), Some("1234"), None),
            1234,
            10,
        );
        assert!(matches!(d, WatchdogDecision::Armed { .. }), "{d:?}");
    }

    #[test]
    fn a_watchdog_pid_naming_another_process_is_inert() {
        let d = resolve(
            &env(Some("/run/systemd/notify"), Some("999"), None),
            1234,
            10,
        );
        assert_eq!(
            d,
            WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                dexd_pid: 1234,
                unit: "999".to_string(),
            })
        );
    }

    #[test]
    fn a_watchdog_pid_that_does_not_parse_is_treated_as_another_process() {
        let d = resolve(
            &env(Some("/run/systemd/notify"), Some("not-a-pid"), None),
            1234,
            10,
        );
        assert_eq!(
            d,
            WatchdogDecision::Inert(InertReason::WatchdogPidMismatch {
                dexd_pid: 1234,
                unit: "not-a-pid".to_string(),
            })
        );
    }

    #[test]
    fn watchdog_usec_with_ample_margin_arms_with_no_warning() {
        // A 180s window (systemd exports microseconds) at a 10s cadence is
        // 18 pings per window, over systemd's twice-per-window guidance.
        let d = resolve(
            &env(Some("/run/systemd/notify"), None, Some("180000000")),
            1234,
            10,
        );
        match d {
            WatchdogDecision::Armed {
                window_secs,
                warning,
                ..
            } => {
                assert_eq!(window_secs, Some(180));
                assert_eq!(warning, None);
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn watchdog_usec_below_twice_the_cadence_warns() {
        // A 15s window at a 10s cadence is 1.5 pings per window, under
        // systemd's twice-per-window guidance.
        let d = resolve(
            &env(Some("/run/systemd/notify"), None, Some("15000000")),
            1234,
            10,
        );
        match d {
            WatchdogDecision::Armed {
                window_secs,
                warning,
                ..
            } => {
                assert_eq!(window_secs, Some(15));
                assert_eq!(
                    warning,
                    Some(ArmedWarning::WindowTooShortForCadence {
                        window_secs: 15,
                        tick_secs: 10
                    })
                );
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn watchdog_usec_at_twice_the_cadence_does_not_warn() {
        // The boundary: a 20s window at a 10s cadence meets systemd's
        // twice-per-window guidance.
        let d = resolve(
            &env(Some("/run/systemd/notify"), None, Some("20000000")),
            1234,
            10,
        );
        match d {
            WatchdogDecision::Armed { warning, .. } => assert_eq!(warning, None),
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    #[test]
    fn a_watchdog_usec_that_does_not_parse_arms_with_no_window_and_no_warning() {
        let d = resolve(
            &env(Some("/run/systemd/notify"), None, Some("soon")),
            1234,
            10,
        );
        match d {
            WatchdogDecision::Armed {
                window_secs,
                warning,
                ..
            } => {
                assert_eq!(window_secs, None);
                assert_eq!(warning, None);
            }
            other => panic!("expected Armed, got {other:?}"),
        }
    }

    // ---- the wire payload ----------------------------------------------

    #[test]
    fn ping_payload_is_watchdog_1_with_no_trailing_newline() {
        assert_eq!(WATCHDOG_PING_PAYLOAD, b"WATCHDOG=1");
    }

    // ---- interpret_send_result: Ok/Err to outcome ----------------------

    #[test]
    fn a_successful_send_reports_sent() {
        assert_eq!(interpret_send_result(Ok(10)), PingOutcome::Sent);
    }

    #[test]
    fn a_send_that_would_block_reports_dropped() {
        let err = io::Error::from(io::ErrorKind::WouldBlock);
        assert_eq!(interpret_send_result(Err(err)), PingOutcome::Dropped);
    }

    #[test]
    fn any_other_send_failure_also_reports_dropped() {
        // For example the notify socket path vanishing mid-run. Every
        // failure is "try again next tick"; none is special-cased.
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

    // ---- send_ping(): real sockets -------------------------------------

    /// Build a socket path under `/tmp`, unique per process and per call.
    /// `AF_UNIX` caps a path at the size of `sockaddr_un.sun_path` (104 bytes
    /// on macOS, 108 on Linux) and the macOS temp directory alone can
    /// overrun that, so keep the prefix short and do not switch this to
    /// `std::env::temp_dir()`.
    /// See docs/design/development.md#portability-details.
    fn unique_socket_path(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::path::PathBuf::from(format!(
            "/tmp/dwd-{:x}-{n:x}-{label}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn send_ping_delivers_the_payload_to_a_real_receiver() {
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
        // Fills the receiver's default buffer within a bounded number of
        // sends. Nothing shrinks SO_RCVBUF first: the crate forbids unsafe
        // code, and std offers no safe setsockopt.
        let path = unique_socket_path("flood");
        let _ = std::fs::remove_file(&path);
        let receiver = UnixDatagram::bind(&path).expect("bind receiver");
        // The receiver is never read, so its queue fills.

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
            "send_to_addr did not report WouldBlock (-> Dropped) within {ATTEMPTS} sends against \
             a receiver that is never read: either this platform's default SO_RCVBUF is far \
             larger than expected, or the sender is no longer non-blocking (a socket that \
             ignores set_nonblocking hangs this loop instead of reaching this assertion)"
        );
    }
}
