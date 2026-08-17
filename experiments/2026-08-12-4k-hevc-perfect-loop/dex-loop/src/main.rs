//! Gapless HEVC looper for the Raspberry Pi: libmpv fed by an endless byte stream.
//!
//! # Why this exists
//!
//! Every mpv looping mechanism stalls at the wrap, because each one makes the
//! decoder re-enter the file. Measured on a Pi 4 at 4K30 and 1080p60 against an
//! HDMI capture card (2026-08-13/14):
//!
//! | mechanism                        | hold at the wrap        |
//! |----------------------------------|-------------------------|
//! | `--loop-file=inf`                | 83 ms, every loop       |
//! | `--ab-loop-a/b`                  | 83 ms, identical        |
//! | `--playlist` + `--prefetch`      | 117-133 ms, worse       |
//! | `ffmpeg -stream_loop` + vout_drm | 67-217 ms, 3 per loop   |
//! | `--loop-file=inf` on raw `.265`  | freezes on the last frame |
//!
//! The only configuration with **zero** held frames is one where the decoder
//! never reaches EOF. That works because the asset has a closed GOP with an IDR
//! at frame 0, so presenting byte 0 straight after the last byte is an ordinary
//! mid-stream IDR rather than a seek.
//!
//! `while true; do cat loop.265; done | mpv -` proves it (verified seamless:
//! zero held frames across 19 wraps at 4K30, and a 3.5 h soak with flat memory)
//! but is not shippable: a process per loop (~29k/day for a 3 s card), and a
//! SIGPIPE hot-spin burning a core if mpv ever exits.
//!
//! A Python version of this same design displayed correctly but ran at **0.6x
//! realtime**, with frames held at random points across the loop -- the
//! signature of a starved feed. The bytes have to arrive at ~5 MB/s against hard
//! per-frame deadlines, and a ctypes callback under the GIL cannot promise that.
//! libmpv's `stream_cb` is a C API, so in Rust the same design is a memcpy.
//!
//! # The whole trick
//!
//! [`read_fn`] never returns 0. Returning 0 means EOF to mpv; wrapping the
//! offset back to the start instead means the stream simply never ends.
//!
//! # Requirements
//!
//! * A raw Annex-B HEVC elementary stream, not MP4:
//!   `ffmpeg -i card.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc loop.265`
//! * The real frame rate, because a raw stream carries no timestamps. This is
//!   why frame rate has to become ingest metadata (milestone 3).

#![deny(unsafe_op_in_unsafe_fn)]

use dex_loop::chunk::{clamp_want, next_chunk};
use dex_loop::exhibit::{
    check_cmdline_matches, mode_resolution, resolve_display, sysfs_modes_contains, DisplaySource,
    ExhibitConfig, DEFAULT_EXHIBIT_CONFIG_PATH,
};
use dex_loop::ffi_consts::{
    MPV_END_FILE_REASON_STOP, MPV_ERROR_UNSUPPORTED, MPV_EVENT_COMMAND_REPLY, MPV_EVENT_END_FILE,
    MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_QUEUE_OVERFLOW,
    MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE, MPV_FORMAT_DOUBLE, MPV_FORMAT_INT64,
    MPV_FORMAT_NONE,
};
use dex_loop::health::{ForceRecoveryTrigger, HealthAction, HealthMonitor};
use dex_loop::heartbeat::{HeartbeatSnapshot, ObservedCounter, PositionSample};
use dex_loop::nal::validate_leading_nals;
use dex_loop::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar};
use dex_loop::watchdog::{self, PingOutcome, WatchdogDecision, WatchdogEnv};
use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs;
use std::os::unix::net::{SocketAddr as UnixSocketAddr, UnixDatagram};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ---------------------------------------------------------------------------
// libmpv FFI. Only the handful of entry points this program needs, transcribed
// from mpv/client.h and mpv/stream_cb.h. Hand-written rather than bindgen: the
// surface is small, stable, and a build-time codegen dependency would outweigh
// it on a device that has to build offline.
// ---------------------------------------------------------------------------

#[repr(C)]
struct MpvHandle {
    _private: [u8; 0],
}

/// `mpv_stream_cb_info` — the callbacks mpv will use for one opened stream.
#[repr(C)]
struct MpvStreamCbInfo {
    cookie: *mut c_void,
    read_fn: Option<extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek_fn: Option<extern "C" fn(*mut c_void, i64) -> i64>,
    size_fn: Option<extern "C" fn(*mut c_void) -> i64>,
    close_fn: Option<extern "C" fn(*mut c_void)>,
    cancel_fn: Option<extern "C" fn(*mut c_void)>,
}

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

/// `mpv_event_end_file`. Only the first two fields are read; the trailing
/// playlist fields exist in the C struct but are irrelevant to a single-file
/// appliance, and reading a prefix of a #[repr(C)] struct is well-defined.
#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct MpvEventLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

/// `mpv_event_property`. Only read when `format == MPV_FORMAT_DOUBLE` (F1's
/// `time-pos` subscription); `data` is a tagged union whose true type
/// depends on `format`, so the tag MUST be checked before `data` is ever
/// dereferenced -- see the MPV_FORMAT_DOUBLE doc comment in ffi_consts.rs.
#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[link(name = "mpv")]
extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(ctx: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut MpvHandle);
    fn mpv_set_option_string(
        ctx: *mut MpvHandle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *const *const c_char) -> c_int;
    // Async counterpart of mpv_command: queues the command and returns
    // immediately (client.h), replying later via MPV_EVENT_COMMAND_REPLY.
    // F1's in-place recovery uses this, never the synchronous mpv_command,
    // specifically so the event thread cannot block on it -- see
    // src/health.rs's module doc.
    fn mpv_command_async(
        ctx: *mut MpvHandle,
        reply_userdata: u64,
        args: *const *const c_char,
    ) -> c_int;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_request_log_messages(ctx: *mut MpvHandle, min_level: *const c_char) -> c_int;
    fn mpv_stream_cb_add_ro(
        ctx: *mut MpvHandle,
        protocol: *const c_char,
        user_data: *mut c_void,
        open_fn: Option<extern "C" fn(*mut c_void, *mut c_char, *mut MpvStreamCbInfo) -> c_int>,
    ) -> c_int;
    // Non-blocking (client.h): queues a subscription and returns
    // immediately. F1's health check calls this exactly ONCE at startup and
    // thereafter only ever reads the position out of the resulting
    // MPV_EVENT_PROPERTY_CHANGE events -- never a synchronous property
    // read -- see src/health.rs's module doc for why that distinction is
    // the whole point.
    fn mpv_observe_property(
        ctx: *mut MpvHandle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    // DELIBERATELY ABSENT: mpv_get_property_string / mpv_get_property (and
    // the mpv_free they require). Every synchronous property read goes
    // through run_locked -> mp_dispatch_lock (player/client.c:1059,
    // misc/dispatch.c:364), which waits WITHOUT A TIMEOUT until the core
    // thread is trapped in its dispatch loop -- a core wedged in a DRM
    // ioctl never gets there, so the caller blocks forever. That is F9:
    // the heartbeat's own diagnostic becoming bug #1 (alive, supervisor
    // green, screen black). Values come from mpv_observe_property +
    // MPV_EVENT_PROPERTY_CHANGE instead. Do not re-add these bindings.
}

// ---------------------------------------------------------------------------
// The endless stream
// ---------------------------------------------------------------------------

/// One reader's position within the looping payload.
///
/// The payload is `&'static [u8]` because it is leaked once at startup and must
/// outlive every mpv thread; there is no meaningful point at which freeing it
/// would be correct while the player runs.
struct LoopStream {
    data: &'static [u8],
    pos: usize,
}

// The cookie is created on one mpv thread, read on the demux thread, and freed
// on whichever thread closes the stream. That requires Send. It is Send today,
// but the raw-pointer laundering through `cookie` means the compiler never
// checks -- so assert it, and a future field (Rc, *mut, an mmap guard) becomes
// a compile error rather than a data race.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<LoopStream>();
};

/// Completed passes over the payload — incremented by `read_fn` on the demux
/// thread each time the position wraps to 0, read by the heartbeat on the
/// event thread. This counts DEMUXER passes, which run ~1 s (readahead)
/// ahead of what is on screen. Relaxed ordering: a monotonic diagnostic
/// counter, not a synchronization point.
static WRAP_COUNT: AtomicU64 = AtomicU64::new(0);

/// Heartbeat cadence: frequent enough to bound "when did it die" to a useful
/// journal window, rare enough to cost nothing.
const HEARTBEAT_SECS: u64 = 600;

/// F1 tier-0 health-check cadence: "order 10 s" per PLAN.md -- low enough
/// frequency, off the decode path, to cost nothing; the escalation policy
/// itself (dex_loop::health::HealthMonitor) requires TWO consecutive
/// non-advancing checks before acting, so real detection latency for an
/// actual stall is roughly 2x this.
const HEALTH_CHECK_SECS: u64 = 10;

/// F1's cumulative, process-lifetime in-place-recovery budget -- see
/// dex_loop::health's module doc ("why the budget never resets") for the
/// full reasoning. 3 is small enough to guarantee the worst case is bounded
/// (at most 3 loadfile-reload attempts, ever, before conceding to tier 1)
/// yet large enough to absorb a handful of isolated transient glitches
/// (HDMI blink, a sink waking late) over a multi-week unattended run
/// without needlessly forcing a full process restart for something tier 0
/// could fix in place.
const MAX_RECOVERY_ATTEMPTS: u32 = 3;

/// `reply_userdata` tag for F1's `time-pos` subscription, so its
/// MPV_EVENT_PROPERTY_CHANGE events are told apart from any property this
/// program observes in the future.
const HEALTH_CHECK_USERDATA: u64 = 1;

/// `reply_userdata` tag for F1's in-place-recovery `loadfile` command, so
/// its MPV_EVENT_COMMAND_REPLY is told apart from any async command this
/// program issues in the future.
const RECOVERY_COMMAND_USERDATA: u64 = 2;

/// `reply_userdata` tags for F9's two observed drop counters, told apart
/// from HEALTH_CHECK_USERDATA/RECOVERY_COMMAND_USERDATA and from each other
/// so the MPV_EVENT_PROPERTY_CHANGE handler never has to `CStr`-compare
/// `p.name` to know which counter a payload belongs to.
const FRAME_DROPS_USERDATA: u64 = 3;
const VO_DELAYED_USERDATA: u64 = 4;

/// The stream read callback: a thin unsafe shell over
/// [`dex_loop::chunk::next_chunk`], which owns (and tests) every rule that
/// matters — never return 0 (to mpv, 0 is final EOF, the one event this
/// program exists to prevent), wrap eagerly, saturate the u64 request size.
/// This function only performs the memcpy the pure core cannot.
///
/// The `None` (zero-length request) branch below is unreachable in mpv 0.40
/// (`stream.c` guards `len <= 0` before ever calling in) and, if it ever did
/// fire, a negative return is treated identically to 0 by
/// `stream_read_unbuffered` (`res <= 0` -> EOF either way) -- so returning an
/// error here is not a mechanism mpv honors specially. It is kept as a
/// defensive sentinel so THIS crate's own diagnostics can tell "asked for
/// nothing" apart from "ran out of things to give"; if the path ever does
/// fire on a future mpv, the result is an ordinary END_FILE -> fatal exit ->
/// supervisor restart, not a seam.
extern "C" fn read_fn(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    // SAFETY: `cookie` is the Box<LoopStream> leaked in `open_fn`, and mpv
    // guarantees it is passed back unmodified for the life of the stream.
    // The `&mut` additionally requires exclusivity. That comes from mpv's
    // STREAM LAYER, not from any serialization guarantee in stream_cb.h --
    // verified against mpv v0.40.0's include/mpv/stream_cb.h: it documents
    // no such thing, and the one callback whose threading it DOES document
    // (cancel_fn) is explicitly cross-thread ("will be called from a
    // separate thread than the demux thread", stream_cb.h:154-155). The
    // real mechanism: a stream_t is single-owner and driven by one thread at
    // a time -- open_cb runs the seek_fn(cookie, 0) probe during open,
    // before fill_buffer/close are ever installed (stream/stream_cb.c), so
    // open-time access happens-before every read, and close (after demux
    // teardown) happens-after the last one. `cancel_fn` is `None` today
    // (see `open_fn` below) specifically so nothing can touch this cookie
    // from that documented second thread; if a future change ever needs
    // `cancel_fn`, this exclusivity argument breaks and the cookie needs a
    // redesign (e.g. an atomic/lock) before it is wired up.
    let s = unsafe { &mut *(cookie as *mut LoopStream) };

    let Some(c) = next_chunk(s.data.len(), s.pos, clamp_want(nbytes)) else {
        // Zero-length request (or an impossible empty payload). Report an
        // error, never 0 -- see the doc comment above for why this is
        // belt-and-braces rather than load-bearing against mpv itself.
        return i64::from(MPV_ERROR_UNSUPPORTED);
    };

    // SAFETY: mpv guarantees `buf` is writable for `nbytes` bytes; next_chunk
    // guarantees c.n >= 1, c.n <= nbytes (the request is clamped, never
    // grown) and c.start + c.n <= data.len(), and the ranges cannot overlap.
    //
    // `.cast::<u8>()` rather than `as *mut u8`, because `c_char` is NOT the same
    // type on both machines this crate is built on: it is `i8` on macOS/aarch64
    // and `u8` on Linux/aarch64. So `buf as *mut u8` is a real conversion on the
    // dev Mac and a no-op on the Pi -- where clippy then rejects it as an
    // unnecessary cast. `.cast()` is correct and lint-clean on both. Do not
    // "simplify" it to `buf`: that only compiles on the Pi.
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(c.start), buf.cast::<u8>(), c.n);
    }
    s.pos = c.next_pos;
    if c.next_pos == 0 {
        // The copy reached the payload's end: one full pass completed.
        WRAP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    c.n as i64
}

/// Report the stream as unseekable, exactly like a pipe.
///
/// Deliberate: an mpv that believes it can seek will try to, and seeking is the
/// operation that produces the seam. Refusing here keeps the only available
/// behaviour "keep reading forwards".
extern "C" fn seek_fn(_cookie: *mut c_void, _offset: i64) -> i64 {
    i64::from(MPV_ERROR_UNSUPPORTED)
}

/// Report the size as unknown, again like a pipe.
///
/// Returning the payload length would let mpv compute a duration and a progress
/// position for a stream that has neither, and would invite it to treat the end
/// of the buffer as the end of the media.
extern "C" fn size_fn(_cookie: *mut c_void) -> i64 {
    i64::from(MPV_ERROR_UNSUPPORTED)
}

extern "C" fn close_fn(cookie: *mut c_void) {
    // SAFETY: reclaims the Box leaked in `open_fn`; mpv calls this exactly once.
    unsafe { drop(Box::from_raw(cookie as *mut LoopStream)) };
}

/// Open callback for the `loop://` protocol. The URI is ignored: the payload is
/// fixed at startup, so there is nothing to parse and nothing that can fail here.
extern "C" fn open_fn(
    user_data: *mut c_void,
    _uri: *mut c_char,
    info: *mut MpvStreamCbInfo,
) -> c_int {
    // SAFETY: `user_data` is the &'static [u8] passed to mpv_stream_cb_add_ro.
    let data: &'static [u8] = unsafe { *(user_data as *mut &'static [u8]) };
    let stream = Box::new(LoopStream { data, pos: 0 });

    // SAFETY: mpv provides a valid, writable info struct for us to fill.
    unsafe {
        (*info).cookie = Box::into_raw(stream) as *mut c_void;
        (*info).read_fn = Some(read_fn);
        (*info).seek_fn = Some(seek_fn);
        (*info).size_fn = Some(size_fn);
        (*info).close_fn = Some(close_fn);
        (*info).cancel_fn = None;
    }
    0
}

// ---------------------------------------------------------------------------

fn err(ctx: *mut MpvHandle, what: &str, code: c_int) -> String {
    let _ = ctx;
    // SAFETY: mpv_error_string returns a static NUL-terminated string.
    let msg = unsafe { CStr::from_ptr(mpv_error_string(code)) };
    format!("{what}: {} ({code})", msg.to_string_lossy())
}

fn set_opt(ctx: *mut MpvHandle, name: &str, value: &str) -> Result<(), String> {
    let n = CString::new(name).map_err(|e| e.to_string())?;
    let v = CString::new(value).map_err(|e| e.to_string())?;
    let rc = unsafe { mpv_set_option_string(ctx, n.as_ptr(), v.as_ptr()) };
    if rc < 0 {
        return Err(err(ctx, &format!("set {name}={value}"), rc));
    }
    Ok(())
}

/// Pi SoC temperature in millidegrees C, if the kernel exposes it. Absent on
/// non-Linux and never an error: the heartbeat degrades to n/a.
fn read_temp_millicelsius() -> Option<i64> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Register one property observer. Non-blocking by construction:
/// `mpv_observe_property` only takes the client-local lock and wakes the
/// core (player/client.c:1536-1572). Returns whether the subscription is
/// live for this run; a failure is never fatal, but per design principle 2
/// it is always loud.
fn observe(ctx: *mut MpvHandle, userdata: u64, name: &str, format: c_int) -> bool {
    let Ok(n) = CString::new(name) else { return false };
    let rc = unsafe { mpv_observe_property(ctx, userdata, n.as_ptr(), format) };
    if rc < 0 {
        eprintln!("warning: {}", err(ctx, &format!("mpv_observe_property({name})"), rc));
    }
    rc >= 0
}

/// One heartbeat line to stderr. Runs on the event thread, off the decode
/// path. Takes no `*mut MpvHandle` -- see the module doc on
/// `dex_loop::heartbeat` for why that absence is the point of F9: with no
/// mpv handle in scope, it is type-level impossible for this function to
/// call into mpv. Every value it prints was already learned from an event.
fn emit_heartbeat(
    started: Instant,
    last_position: Option<(f64, Instant)>,
    frame_drops: ObservedCounter,
    vo_delayed: ObservedCounter,
    watchdog_pings_dropped: Option<u64>,
) {
    eprintln!(
        "{}",
        HeartbeatSnapshot {
            wraps: WRAP_COUNT.load(Ordering::Relaxed),
            uptime_secs: started.elapsed().as_secs(),
            temp_millicelsius: read_temp_millicelsius(),
            position: last_position.map(|(secs, at)| PositionSample {
                secs,
                age_secs: at.elapsed().as_secs(),
            }),
            frame_drops,
            vo_delayed,
            watchdog_pings_dropped,
        }
        .render()
    );
}

/// F10: the live systemd-watchdog resources held across the whole run --
/// the non-blocking socket (opened once, never recreated) plus the address
/// it pings and a running count of drops for the heartbeat line. `None`
/// (via [`setup_watchdog`]) for every non-systemd run: Mac dev, bench tmux,
/// CI, any manual invocation off systemd -- see `dex_loop::watchdog`'s
/// module doc.
///
/// This struct -- and the actual `send_to_addr` call site -- deliberately
/// live HERE, in the driver, not in the library: `dex_loop::watchdog`
/// already owns everything decidable (the handshake, the address
/// construction, the non-blocking send wrapper, all unit-tested with real
/// sockets under `cargo test --lib`, see that module's tests). What is left
/// is process-lifetime resource ownership and the "when do we call this"
/// wiring -- exactly what `main.rs` already does for every other piece of
/// mpv-facing state in this program (`health`, `frame_drops`, `vo_delayed`),
/// per `lib.rs`'s own "main.rs is a thin driver" principle.
struct WatchdogRuntime {
    socket: UnixDatagram,
    addr: UnixSocketAddr,
    pings_dropped: u64,
}

/// One policy for every "resolve() said Armed but the ping socket cannot be
/// established" arm of [`setup_watchdog`]. Which way it goes depends on
/// whether systemd's kill timer is demonstrably running (`$WATCHDOG_USEC`
/// present -- systemd exports it exactly when `WatchdogSec=` is configured):
///
/// - **Timer armed: `exit(1)` now.** Nothing this process logs can disarm
///   the timer on systemd's side -- "running without pings" under an armed
///   `WatchdogSec=` means being SIGABRT-killed every window, forever (a ~2 s
///   black hiccup every 3 min under the shipped unit), while a
///   "watchdog DISABLED" journal line actively hides the cause of every one
///   of those kills. `Restart=always` + `RestartSec=2` retries in seconds,
///   and this failure class (address resolution, fd exhaustion,
///   `set_nonblocking`) is transient -- a clean fast retry strictly beats a
///   `WatchdogSec`-cadence kill loop with a lying journal line.
/// - **No timer: run without pings.** Nothing will kill us for not pinging,
///   so pings are genuinely pointless; be loud once and play the asset.
fn watchdog_setup_failed(
    cause: &str,
    kill_timer_armed: bool,
    window_secs: Option<u64>,
) -> Option<WatchdogRuntime> {
    if kill_timer_armed {
        let window =
            window_secs.map(|w| format!("{w}s")).unwrap_or_else(|| "WatchdogSec".to_string());
        eprintln!(
            "error: dex-loop: watchdog: {cause} -- systemd's WatchdogSec timer IS armed \
             ($WATCHDOG_USEC is set) and cannot be disarmed from inside this process: without \
             pings, systemd would SIGABRT this process every {window} while it plays normally. \
             Exiting now so Restart= retries cleanly instead."
        );
        std::process::exit(1);
    }
    eprintln!(
        "warning: dex-loop: watchdog: {cause} -- no WatchdogSec timer is armed ($WATCHDOG_USEC \
         absent), so pings would prove nothing; running WITHOUT watchdog pings for this run"
    );
    None
}

/// Resolve the systemd watchdog handshake and, if armed, open the
/// non-blocking socket it needs. Failures along the way (address resolution,
/// socket creation, `set_nonblocking`) follow [`watchdog_setup_failed`]'s
/// policy: they only downgrade to "no pings" when systemd's own kill timer
/// is NOT running -- when it is, no in-process downgrade exists (the timer
/// keeps counting regardless of what we log), so the process exits for a
/// clean fast retry rather than limping into a guaranteed
/// `WatchdogSec`-cadence kill loop.
fn setup_watchdog(tick_secs: u64) -> Option<WatchdogRuntime> {
    let env = WatchdogEnv::from_process_env();
    // systemd exports $WATCHDOG_USEC exactly when WatchdogSec= is configured
    // on the unit -- its presence means a kill timer is counting RIGHT NOW,
    // no matter what this process does or logs about its own pings.
    let kill_timer_armed = env.watchdog_usec.is_some();
    let decision = watchdog::resolve(&env, std::process::id(), tick_secs);
    let (addr, window_secs, warning) = match decision {
        WatchdogDecision::Inert(reason) => {
            eprintln!("dex-loop: watchdog: inert ({reason})");
            // The pid-mismatch arm is a NORMAL inert condition when no timer
            // is armed -- but under an armed WatchdogSec= it is a death
            // sentence on a schedule: systemd expects pings from the unit's
            // main pid, this process (rightly) refuses to ping under someone
            // else's identity, and the timer fires every window regardless.
            // Say so, so the journal explains the SIGABRT kills that follow
            // (e.g. a future edit wrapping ExecStart in a shell).
            if kill_timer_armed
                && matches!(reason, watchdog::InertReason::WatchdogPidMismatch { .. })
            {
                eprintln!(
                    "warning: dex-loop: watchdog: $WATCHDOG_USEC is set, so systemd's WatchdogSec \
                     timer IS armed and expects pings from the unit's MAIN pid -- with none \
                     arriving, systemd will kill this process every watchdog window; expect a \
                     kill/restart loop until the unit is fixed (is ExecStart wrapped in a shell?)"
                );
            }
            return None;
        }
        WatchdogDecision::Armed { addr, window_secs, warning } => (addr, window_secs, warning),
    };

    let sockaddr = match watchdog::socket_addr(&addr) {
        Ok(a) => a,
        Err(e) => {
            return watchdog_setup_failed(
                &format!("cannot resolve $NOTIFY_SOCKET address ({addr:?}): {e}"),
                kill_timer_armed,
                window_secs,
            );
        }
    };
    let socket = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(e) => {
            return watchdog_setup_failed(
                &format!("UnixDatagram::unbound failed: {e}"),
                kill_timer_armed,
                window_secs,
            );
        }
    };
    // LOAD-BEARING: see dex_loop::watchdog's module doc §"no new blocking
    // call" -- a blocking send against a full receiver queue would be a new
    // way for THIS feature to hang the event thread, exactly the class of
    // bug F9 already had to fix once for the heartbeat.
    if let Err(e) = socket.set_nonblocking(true) {
        return watchdog_setup_failed(
            &format!("set_nonblocking failed: {e} (refusing a socket that could block the event thread)"),
            kill_timer_armed,
            window_secs,
        );
    }

    let window = window_secs.map(|w| format!("{w}s")).unwrap_or_else(|| "unknown".to_string());
    eprintln!("dex-loop: watchdog: armed (window {window}, ping cadence {tick_secs}s)");
    if let Some(w) = warning {
        eprintln!("warning: dex-loop: watchdog: {w}");
    }

    Some(WatchdogRuntime { socket, addr: sockaddr, pings_dropped: 0 })
}

/// Whether an `MPV_EVENT_END_FILE` with this `reason` is an expected
/// teardown half of F1's in-place recovery -- its own `loadfile ...
/// replace` command(s) -- rather than a real failure. Pure and separately
/// tested (unlike the rest of the event loop, which needs libmpv) so this
/// one condition -- the entire fix for the regression where every recovery
/// attempt killed the process on its own first step -- cannot silently
/// break again without a failing test. See `MPV_END_FILE_REASON_STOP`'s doc
/// comment for the mpv behaviour this encodes.
///
/// `recovery_stops_pending` is a COUNT, not a bool: the organic health-check
/// tick and T7's forced probe can both issue a recovery in the same loop
/// iteration (main.rs's tick block runs before the trigger block), and
/// `mpv_command_async` only QUEUES a loadfile against a core that may still
/// be busy from a still-wedged episode, so a second recovery can be issued
/// (and accepted) before the first attempt's stop has been observed. Two
/// in-flight recoveries produce two END_FILE(reason=stop) events; a bool can
/// absorb only the first and would treat the second -- an entirely expected
/// teardown -- as fatal. See PLAN.md's F1 addendum ("overlapping tier-0
/// recoveries") for the confirmed scenario this fixes.
fn is_expected_recovery_stop(recovery_stops_pending: u32, reason: c_int) -> bool {
    recovery_stops_pending > 0 && reason == MPV_END_FILE_REASON_STOP
}

/// Act on a [`HealthAction`], whatever produced it. Shared by the organic
/// health-check tick and T7's `--force-recovery-after-secs` bench probe
/// (`HealthMonitor::force_recovery`) specifically so a forced probe drives
/// the EXACT SAME mpv-facing mechanics -- `mpv_command_async(loadfile ...
/// replace)`, then incrementing `recovery_stops_pending` so the resulting
/// `END_FILE(reason=stop)` is absorbed rather than treated as fatal (see
/// `is_expected_recovery_stop`) -- that a real stall would. That identity is
/// the point of T7: it is what lets a bench probe stand in for a real
/// stall's recovery path at all. `reason` is only the situational log
/// prefix; the two callers differ in WHY a recovery is due, not in what
/// happens once it is.
fn act_on_health_action(
    ctx: *mut MpvHandle,
    action: HealthAction,
    reason: &str,
    last_position: &mut Option<(f64, Instant)>,
    recovery_stops_pending: &mut u32,
) {
    match action {
        HealthAction::Healthy => {}
        HealthAction::AttemptRecovery { attempt, max } => {
            eprintln!(
                "dex-loop: health check: {reason} -- attempting in-place recovery \
                 {attempt}/{max} (re-issuing loadfile: restarts demux+decode and forces \
                 a VO reconfigure; does not tear down/re-init the DRM/GPU context itself \
                 -- see PLAN.md's F1 addendum)"
            );
            // Mirror HealthMonitor's own baseline reset (health.rs:
            // `self.last_position = None`, shared by tick's AttemptRecovery
            // arm and force_recovery via issue_recovery_or_escalate).
            // Without this, the driver would keep feeding the STALE
            // pre-recovery position back into the next tick, which the
            // monitor -- now comparing against its own `None` baseline --
            // would misread as a fresh first sample (i.e. progress), buying
            // a spurious "Healthy" tick that stretches the real escalation
            // timeline. Since F9, `last_position` also carries the sample's
            // `Instant` in the same tuple, so this one line clears
            // `pos-age=`'s staleness clock too.
            *last_position = None;
            // mpv_command_async, not mpv_command: this call runs on the
            // SAME event thread that also has to keep detecting every fatal
            // event, and a synchronous command could block that thread
            // against a wedged core -- see dex_loop::health's module doc.
            let cmd_loadfile = CString::new("loadfile").unwrap();
            let cmd_url = CString::new("loop://endless").unwrap();
            let cmd_replace = CString::new("replace").unwrap();
            let argv: [*const c_char; 4] = [
                cmd_loadfile.as_ptr(),
                cmd_url.as_ptr(),
                cmd_replace.as_ptr(),
                std::ptr::null(),
            ];
            let rc = unsafe { mpv_command_async(ctx, RECOVERY_COMMAND_USERDATA, argv.as_ptr()) };
            if rc < 0 {
                eprintln!("warning: {}", err(ctx, "mpv_command_async(loadfile)", rc));
            } else {
                // Only now: the command was actually queued, so mpv WILL
                // deliver an END_FILE(reason=stop) for the file being
                // replaced (see MPV_END_FILE_REASON_STOP's doc comment) --
                // that event must be absorbed, not treated as the fatal
                // failure it would otherwise look like. If mpv_command_async
                // itself failed (above), no such event is coming, so the
                // count must NOT be incremented. Saturating: bounded in
                // practice by MAX_RECOVERY_ATTEMPTS (the cumulative budget
                // this same command draws from), so saturation never
                // actually engages -- it is here so a future change to that
                // relationship fails safe (an undercount that stays fatal)
                // rather than wrapping into a silent lie.
                *recovery_stops_pending = recovery_stops_pending.saturating_add(1);
            }
        }
        HealthAction::Escalate => {
            eprintln!(
                "dex-loop: FATAL: tier-0 self-healing exhausted its recovery budget \
                 ({MAX_RECOVERY_ATTEMPTS} attempt(s)) with no progress ({reason}) -- \
                 exiting so the supervisor restarts (tier 1)"
            );
            // std::process::exit, not `break` into the shared
            // mpv_terminate_destroy() teardown at the end of main: Escalate
            // fires precisely because the core looks wedged (or, for a
            // forced probe, to prove the SAME exit path an organic
            // escalation would take), and mpv_terminate_destroy
            // synchronously joins mpv's own threads -- a core that cannot
            // advance time-pos may not be able to complete that join
            // either, which would block the one exit path whose entire job
            // is to let the supervisor take over. Per design principle 4
            // (the mains switch IS the shutdown path), exiting abruptly
            // here is not a shortcut, it is correct.
            //
            // F10 INVARIANT: this `eprintln!` line above -- or anything
            // else added to this arm before `exit(1)` -- can in principle
            // block (e.g. against a wedged journald), and that possibility
            // is EXACTLY what the systemd watchdog (`WatchdogSec=` in
            // deploy/dex-loop.service, dex_loop::watchdog) exists to catch:
            // if this arm hangs here, the tick loop never completes another
            // iteration, so no further WATCHDOG=1 pings are sent, and
            // systemd's timer fires. Do NOT add a "final ping" to this arm
            // to try to look more alive on the way out -- that would reset
            // the watchdog's countdown right before the one hang this
            // feature exists to catch, defeating it. See
            // dex_loop::watchdog's module doc for the full argument.
            std::process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: dex-loop <stream.265> [--fps <F>] [--mode WxH@R] [--bench-no-sidecar] [--no-defaults] [--opt K=V ...]

  <stream.265>        raw Annex-B HEVC elementary stream, looped endlessly
  <stream.265>.json   ingest sidecar, REQUIRED: {{\"fps\":\"30\",\"sha256\":\"<64 hex>\"}}
                      fps comes from it; the sha256 must match the asset bytes
  --fps F             optional cross-check; must equal the sidecar fps exactly
  --mode WxH@R        cross-check against the exhibit config's display_mode (F6);
                      optional alongside --bench-no-sidecar, where it is the only
                      source instead (defaults to auto there)
  --exhibit-config PATH
                      F6: path to the exhibit config (default: {DEFAULT_EXHIBIT_CONFIG_PATH}).
                      Binds the display mode, the expected KMS force, and the
                      connector -- see man dex-exhibit-apply.
  --bench-no-sidecar  BENCH ONLY: skip the sidecar AND the exhibit config, take
                      --fps/--mode as given
  --force-recovery-after-secs N
                      T7 BENCH ONLY: force a tier-0 in-place recovery N seconds after
                      the loadfile request (NOT N seconds of confirmed playback --
                      decode startup takes time too), whether or not anything has
                      stalled. For a live-fire run meant to catch mid-playback issues
                      rather than startup ones, pick N with margin over real decode
                      startup latency. REQUIRES --bench-no-sidecar (refused otherwise)
                      so it can never fire against a real, sidecar-bound deployment
                      asset.
  --bench-wedge-after-secs N
                      F10 BENCH ONLY: N seconds after startup, deliberately hang the
                      event thread FOREVER -- simulates the one hazard class F1/F9
                      cannot see (an event-thread hang outside any mpv call), to prove
                      whether a systemd watchdog (WatchdogSec=) actually fires and
                      restarts this process. The process never recovers on its own once
                      this fires; only an external actor (systemd, or a test harness's
                      own kill) can end it. REQUIRES --bench-no-sidecar (refused
                      otherwise) so it can never fire against a real, sidecar-bound
                      deployment asset.
  --opt K=V           pass an extra mpv option (repeatable)
  --no-defaults       omit the built-in Pi 4 zero-copy option set

exit codes: 2 = refused before playback (bad invocation/asset/sidecar/display; fix and redeploy)
            1 = playback/runtime failure (the supervisor restarts)"
    );
    std::process::exit(2)
}

/// Locate `/sys/class/drm/card<N>-<connector>/modes`. The card number is not
/// hardcoded: vc4/v3d probe order makes it unstable across kernel versions
/// (the same reason `deploy/dex-wait-hdmi` globs it rather than assuming
/// `card1`). Returns `None` if no such entry exists -- e.g. no DRM at all (a
/// CI container), or a mistyped connector name.
fn find_sysfs_modes_path(connector: &str) -> Option<String> {
    let suffix = format!("-{connector}");
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") && name.ends_with(suffix.as_str()) {
            let modes_path = entry.path().join("modes");
            if modes_path.is_file() {
                return Some(modes_path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn main() -> ExitCode {
    // Identify the build before anything can fail: a field journal that
    // starts with an unidentifiable process is undebuggable weeks later.
    eprintln!(
        "dex-loop {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("DEX_GIT_HASH")
    );

    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }

    let mut path: Option<String> = None;
    let mut cli_fps: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut extra: Vec<(String, String)> = Vec::new();
    let mut defaults = true;
    let mut bench_no_sidecar = false;
    let mut force_recovery_after_secs: Option<u64> = None;
    let mut bench_wedge_after_secs: Option<u64> = None;
    let mut exhibit_config_path: Option<String> = None;
    // Test-only override so integration tests can supply a synthetic kernel
    // cmdline instead of depending on the actual host's /proc/cmdline, which
    // varies by machine (a CI container has no video= token at all; the real
    // bench Pi, once F6 is deployed there, always does). Never printed in
    // usage(): a real deployment always reads the real /proc/cmdline.
    let mut proc_cmdline_path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fps" => {
                i += 1;
                // A missing value here (flag is the last token -- an edited
                // systemd unit, a line-continuation typo) must refuse loudly,
                // not evaporate: a silently-dropped --fps falls through to
                // "no cross-check", and a silently-dropped --mode falls
                // through to the connector-preferred mode -- wrong cadence
                // or wrong resolution, forever, with no error.
                let Some(v) = args.get(i) else { usage() };
                cli_fps = Some(v.clone());
            }
            "--mode" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                mode = Some(v.clone());
            }
            "--exhibit-config" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                exhibit_config_path = Some(v.clone());
            }
            "--proc-cmdline" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                proc_cmdline_path = Some(v.clone());
            }
            "--no-defaults" => defaults = false,
            "--bench-no-sidecar" => bench_no_sidecar = true,
            "--force-recovery-after-secs" => {
                i += 1;
                // Same "refuse loudly, never evaporate" discipline as
                // --fps/--mode above: a dropped value here would silently
                // leave T7 disarmed, which is harmless, but a MALFORMED
                // value (e.g. a typo'd flag argument) must not be read as
                // "flag absent" either -- usage() either way.
                let Some(v) = args.get(i) else { usage() };
                let Ok(n) = v.parse::<u64>() else { usage() };
                force_recovery_after_secs = Some(n);
            }
            "--bench-wedge-after-secs" => {
                i += 1;
                // Same "refuse loudly, never evaporate" discipline as
                // --force-recovery-after-secs above.
                let Some(v) = args.get(i) else { usage() };
                let Ok(n) = v.parse::<u64>() else { usage() };
                bench_wedge_after_secs = Some(n);
            }
            "--opt" => {
                i += 1;
                let kv = args.get(i).cloned().unwrap_or_default();
                match kv.split_once('=') {
                    Some((k, v)) => extra.push((k.to_string(), v.to_string())),
                    None => usage(),
                }
            }
            "-h" | "--help" => usage(),
            s if !s.starts_with('-') && path.is_none() => path = Some(s.to_string()),
            _ => usage(),
        }
        i += 1;
    }

    let Some(path) = path else { usage() };

    // T7 (PLAN.md) -- the "impossible to enable accidentally in a
    // deployment" requirement, enforced as a gate rather than left to
    // operator discipline. `--force-recovery-after-secs` REQUIRES
    // `--bench-no-sidecar`. This is not an arbitrary pairing: it ties the
    // bench-only recovery probe to the SAME escape hatch that already keeps
    // `--bench-no-sidecar` out of every real deployment (deploy/dex-loop.service
    // never passes it -- a real asset is bound to its ingest sidecar, full
    // stop), so a live-fire probe can never end up armed against a gallery
    // show by an operator pasting a bench command line into the wrong
    // place. Checked here, before the asset is even read, so the refusal is
    // unconditional on CLI shape alone -- it does not depend on whether a
    // sidecar happens to exist on disk.
    if force_recovery_after_secs.is_some() && !bench_no_sidecar {
        eprintln!(
            "error: --force-recovery-after-secs requires --bench-no-sidecar -- it is a \
             T7 BENCH-ONLY live-fire probe (PLAN.md) that forces a tier-0 in-place \
             recovery on a timer, whether or not anything has actually stalled, and \
             must never be armed against what could be a real, sidecar-bound \
             deployment asset. Add --bench-no-sidecar --fps <F> to run it on a bench, \
             or drop --force-recovery-after-secs to run normally."
        );
        return ExitCode::from(2);
    }

    // F10 (PLAN.md) -- the identical "impossible to enable accidentally in
    // a deployment" gate as T7 above, for the same reason: a probe that
    // deliberately hangs the event thread forever must never be reachable
    // against a real, sidecar-bound show, however it got pasted into a
    // command line.
    if bench_wedge_after_secs.is_some() && !bench_no_sidecar {
        eprintln!(
            "error: --bench-wedge-after-secs requires --bench-no-sidecar -- it is an F10 \
             BENCH-ONLY probe (PLAN.md) that deliberately hangs the event thread forever to \
             prove whether a systemd watchdog actually fires, and must never be armed against \
             what could be a real, sidecar-bound deployment asset. Add --bench-no-sidecar \
             --fps <F> to run it on a bench, or drop --bench-wedge-after-secs to run normally."
        );
        return ExitCode::from(2);
    }

    // F6 -- the exhibit display config. Runs BEFORE the asset is read: these
    // are the cheapest gates in the program and must not depend on asset
    // presence (a wrong-panel install is worth catching even if the asset
    // path is also wrong). Order: config parse -> cmdline gate -> sysfs mode
    // pre-flight. See src/exhibit.rs module docs and PLAN.md's F6 entry.
    let exhibit_config_path = exhibit_config_path
        .clone()
        .unwrap_or_else(|| DEFAULT_EXHIBIT_CONFIG_PATH.to_string());
    let exhibit_config: Option<ExhibitConfig> = if bench_no_sidecar {
        None
    } else {
        match fs::read_to_string(&exhibit_config_path) {
            Ok(text) => match ExhibitConfig::from_json(&text) {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("error: {exhibit_config_path}: {e}");
                    return ExitCode::from(2);
                }
            },
            // A MISSING file is deferred to resolve_display below, which is
            // the single place that states the "no exhibit config" refusal
            // (mirroring how sidecar::resolve_fps states the analogous "no
            // sidecar" message). Any OTHER read error is a different
            // operational fact and gets its own message here: an
            // EXISTING-but-unreadable file (root-edited and saved 0600, a
            // restored backup with the wrong owner) is fixed by chmod/chown,
            // not by creating a file the operator can see already exists --
            // telling them to create it would be actively misleading, the
            // message class this crate's principles forbid.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                eprintln!(
                    "error: cannot read {exhibit_config_path}: {e} -- the file exists but is \
                     not readable; check its owner/permissions (the dex user must be able to \
                     read it)"
                );
                return ExitCode::from(2);
            }
        }
    };

    let display = match resolve_display(exhibit_config.as_ref(), mode.as_deref(), bench_no_sidecar)
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // The cmdline gate: keeps the exhibit config and the KMS-layer `video=`
    // token honest with each other. Skipped under the bench flag, since a
    // ~/bench build runs on hand-managed boot state by definition (§2.3/§3.3
    // of the F6 design) -- the whole POINT of --bench-no-sidecar is running
    // outside the deployment config's authority.
    if !bench_no_sidecar {
        let proc_cmdline_path = proc_cmdline_path
            .clone()
            .unwrap_or_else(|| "/proc/cmdline".to_string());
        match fs::read_to_string(&proc_cmdline_path) {
            Ok(cmdline) => {
                if let Err(e) = check_cmdline_matches(&cmdline, &display.connector, &display.kms_force)
                {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
                }
            }
            Err(e) => {
                eprintln!("error: cannot read {proc_cmdline_path}: {e}");
                return ExitCode::from(2);
            }
        }
    }

    // The sysfs mode pre-flight: catches "the configured resolution is not
    // even in this connector's mode list" (the wrong-panel case) before any
    // asset/sidecar/NAL gate runs. Plain-text sysfs, no privilege, no libmpv
    // -- `/sys/class/drm/card*-<connector>/modes`, one WxH per line. Runs
    // whenever a specific mode is requested, bench or not: it is a real
    // hardware-agreement check that reads nothing from /etc or /proc, so
    // nothing about "bench" exempts it (unlike the cmdline gate above, which
    // is specifically about /etc vs /boot agreement).
    if let Some(want_wh) = mode_resolution(&display.display_mode) {
        match find_sysfs_modes_path(&display.connector) {
            Some(modes_path) => match fs::read_to_string(&modes_path) {
                Ok(modes_text) => {
                    if !sysfs_modes_contains(&modes_text, want_wh) {
                        let offered: Vec<&str> = modes_text.lines().map(str::trim).collect();
                        eprintln!(
                            "error: display_mode {want_wh:?} is not among the modes {} \
                             ({modes_path}) offers: {offered:?} -- wrong panel, or the \
                             cmdline force (if any) has not taken effect yet (reboot?)",
                            display.connector
                        );
                        return ExitCode::from(2);
                    }
                }
                Err(e) => {
                    eprintln!("error: cannot read {modes_path}: {e}");
                    return ExitCode::from(2);
                }
            },
            None => {
                eprintln!(
                    "error: display_mode {want_wh:?} requested for connector {}, but no sysfs \
                     modes list was found for it under /sys/class/drm -- is the connector name \
                     correct, and is DRM available?",
                    display.connector
                );
                return ExitCode::from(2);
            }
        }
    }

    eprintln!(
        "dex-loop: display {} ({}), connector {}, kms-force {}",
        display.display_mode,
        match display.source {
            DisplaySource::Config => format!("exhibit config {exhibit_config_path}"),
            DisplaySource::Bench => "BENCH OVERRIDE, unbound".to_string(),
        },
        display.connector,
        if bench_no_sidecar {
            "not checked (bench)".to_string()
        } else {
            display.kms_force.clone()
        },
    );

    // Read the loop once. These are small (1.3 MB at 1080p, 14.8 MB at 4K for a
    // 3 s card) and holding it in memory removes the filesystem from the hot
    // path: no re-open, no page-cache dependency, no I/O stall at the wrap.
    let payload = match fs::read(&path) {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => {
            eprintln!("error: {path} is empty");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let leaked: &'static [u8] = Box::leak(payload.into_boxed_slice());

    // F3 — bind the asset to its ingest sidecar. A raw Annex-B stream has no
    // timestamps: a WRONG --fps plays slow/fast forever with zero errors and
    // every metric nominal — the one failure that is undetectable by
    // construction. So the frame rate travels WITH the asset, bound by a
    // sha256, and an unbound asset is refused. `--bench-no-sidecar --fps F`
    // is the deliberate two-flag bench escape hatch.
    let sidecar_path = format!("{path}.json");
    let sidecar: Option<Sidecar> = if bench_no_sidecar {
        None
    } else {
        match fs::read_to_string(&sidecar_path) {
            Ok(text) => match Sidecar::from_json(&text) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("error: {sidecar_path}: {e}");
                    return ExitCode::from(2);
                }
            },
            Err(e) => {
                eprintln!(
                    "error: cannot read sidecar {sidecar_path}: {e}\n\
                     an asset without its ingest sidecar is unbound (fps would be a \
                     guess); re-ingest to produce it, or use --bench-no-sidecar \
                     --fps <F> on a bench"
                );
                return ExitCode::from(2);
            }
        }
    };

    let (fps, fps_source) = match resolve_fps(
        sidecar.as_ref().map(|s| s.fps.as_str()),
        cli_fps.as_deref(),
        bench_no_sidecar,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    if let Some(s) = &sidecar {
        if let Err(e) = verify_payload(leaked, s) {
            eprintln!("error: {path}: {e}");
            return ExitCode::from(2);
        }
    }

    // F6 addendum (2026-08-17 review): display_mode "auto" skips the sysfs
    // pre-flight by construction (no mode to check against), which converts
    // fail-closed into fail-silent on the one config the .deb ships by
    // default -- a forgotten /etc/dex/exhibit.json edit on hardware that
    // builds no 4K mode unforced (the Cam Link case) plays the artwork at
    // whatever the connector negotiates, for weeks, with every metric green.
    // The sidecar is hash-bound to the asset and already names its
    // resolution, so at least SAY SO when the connector cannot even offer
    // it. A warning, not a refusal: whether "auto" should stay the factory
    // default at all (vs an explicit "unset" that refuses like a missing
    // config) is one of the parked F6 design questions -- see PLAN.md.
    if display.display_mode == "auto" {
        if let Some((w, h)) = sidecar.as_ref().and_then(|s| s.width.zip(s.height)) {
            let want = format!("{w}x{h}");
            if let Some(modes_path) = find_sysfs_modes_path(&display.connector) {
                if let Ok(modes_text) = fs::read_to_string(&modes_path) {
                    if !sysfs_modes_contains(&modes_text, &want) {
                        let offered: Vec<&str> = modes_text.lines().map(str::trim).collect();
                        eprintln!(
                            "warning: display_mode is \"auto\" and the asset is {want} (per its \
                             sidecar), but connector {} offers only {offered:?} ({modes_path}) \
                             -- KMS will drive whatever fallback it negotiates and the artwork \
                             will play at the WRONG resolution with every metric green. If this \
                             display needs a forced mode to build {want} (e.g. the Cam Link \
                             builds no 4K mode unforced), set display_mode and kms_force in \
                             /etc/dex/exhibit.json, run 'sudo dex-exhibit-apply', and reboot",
                            display.connector
                        );
                    }
                }
            }
        }
    }

    // F4 — validate the leading NALs. The wrap is only seamless because byte
    // 0 begins VPS/SPS/PPS + IDR; a wrong-but-intact asset (open GOP, no
    // leading IDR, not Annex-B at all) would glitch at every wrap, silently,
    // ~29k times/day. Truncation is caught by the F3 hash above; this catches
    // shape. Runs in bench mode too — the premise holds there as well.
    if let Err(e) = validate_leading_nals(leaked) {
        eprintln!("error: {path}: {e}");
        return ExitCode::from(2);
    }

    eprintln!(
        "dex-loop: {} bytes, fps {fps} ({}), looping endlessly",
        leaked.len(),
        match fps_source {
            FpsSource::Sidecar => "sidecar",
            FpsSource::BenchOverride => "BENCH OVERRIDE, unbound",
        }
    );

    if let Some(n) = force_recovery_after_secs {
        // Loud on purpose (principle 2): the gate above makes this
        // impossible to reach without --bench-no-sidecar already having
        // been accepted, but a run that silently, quietly forces its own
        // recovery mid-show is exactly the kind of surprise that belongs in
        // the journal in giant letters, not inferred later from a
        // recovery log line with no explanation of why it fired.
        eprintln!(
            "warning: BENCH ONLY (T7): --force-recovery-after-secs={n} is ARMED -- this \
             run will FORCE a tier-0 in-place recovery {n}s after the loadfile request was \
             queued (NOT {n}s of confirmed playback -- decode startup can itself take a \
             few seconds, so a small N can fire during startup rather than steady \
             playback), whether or not anything has actually stalled. Never pass this \
             flag on a real deployment asset (see PLAN.md's T7 entry)."
        );
    }

    if let Some(n) = bench_wedge_after_secs {
        // Same "loud on purpose" discipline as T7 above.
        eprintln!(
            "warning: BENCH ONLY (F10 wedge probe): --bench-wedge-after-secs={n} is ARMED -- \
             this run will deliberately hang the event thread FOREVER {n}s after startup, \
             simulating F10's hazard class (an event-thread hang outside any mpv call). \
             Never pass this flag on a real deployment asset (see PLAN.md's F10 entry)."
        );
    }

    let ctx = unsafe { mpv_create() };
    if ctx.is_null() {
        eprintln!("error: mpv_create failed");
        return ExitCode::FAILURE;
    }

    let mut opts: Vec<(String, String)> = Vec::new();
    if defaults {
        // Measured on Pi 4 / trixie. The decoder emits Broadcom SAND-tiled NV12
        // and the display can scan SAND out natively, but ONLY straight onto a
        // KMS plane -- every other path detiles (CPU: 14.3 fps, GL: 5 fps).
        // `drmprime-overlay` is the interop that puts the frame on a plane;
        // plain `drmprime` imports into GL and is 2x slower.
        for (k, v) in [
            ("vo", "gpu"),
            ("hwdec", "drm"),
            ("gpu-context", "drm"),
            ("gpu-api", "opengl"),
            ("gpu-hwdec-interop", "drmprime-overlay"),
            // Video on the primary plane, mpv's GL/OSD surface on the overlay --
            // SWAPPED from mpv's defaults, deliberately: it keeps the 4K video
            // off the V3D render path entirely. Caveat: mpv sets ZPOS only on
            // the video plane, so video-under-GL visibility relies on vc4's
            // default plane ordering rather than anything mpv guarantees.
            // Verified on this Pi 4 + kernel; re-verify after a kernel upgrade
            // or on any other DRM driver.
            ("drm-draw-plane", "overlay"),
            ("drm-drmprime-video-plane", "primary"),
            ("video-sync", "display-resample"),
            // Without this, a decoder that cannot use the hardware path falls
            // back to software SILENTLY and plays 4K30 at ~14 fps. Making it
            // fatal turns an invisible performance collapse into an END_FILE
            // error, which is handled and restartable.
            ("hwdec-software-fallback", "no"),
            ("fullscreen", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
            // A raw elementary stream has no timestamps; mpv must generate them.
            ("correct-pts", "no"),
            // NOTE: this is belt-and-braces, not the load-bearing bound. mpv
            // only runs its aggressive cache for streams flagged as network,
            // and stream_cb streams are not; readahead here is governed by
            // demuxer-readahead-secs instead. The flat memory measured over
            // 3.5 h is due to that, not to this cap.
            ("demuxer-max-bytes", "64MiB"),
            // The actual prefetch depth. One second of decoded-ahead insurance
            // across the wrap, where the whole gaplessness claim is decided.
            ("demuxer-readahead-secs", "1.0"),
        ] {
            opts.push((k.to_string(), v.to_string()));
        }
    }
    opts.push(("container-fps-override".into(), fps));
    // F6 -- the resolved exhibit display binding. "auto" means: pass no
    // drm-mode at all, which is mpv's own documented default
    // (drm-mode=preferred) -- see exhibit::resolve_display's docs on what
    // "auto" precisely means. drm-connector is passed unconditionally: every
    // ResolvedDisplay carries a connector (defaulted if not configured), so
    // this is always explicit rather than relying on mpv's own connector
    // pick, which the pre-F6 code never stated either way.
    if display.display_mode != "auto" {
        opts.push(("drm-mode".into(), display.display_mode.clone()));
    }
    opts.push(("drm-connector".into(), display.connector.clone()));
    opts.extend(extra);

    for (k, v) in &opts {
        if let Err(e) = set_opt(ctx, k, v) {
            eprintln!("error: {e}");
            unsafe { mpv_terminate_destroy(ctx) };
            // A rejected option is deterministic given these inputs: the
            // same asset + flags fail identically on every restart, so per
            // the exit-code contract this is "bad invocation" (2) -- fix and
            // redeploy -- not a runtime failure (1) that the supervisor's
            // restart loop could ever resolve on its own.
            return ExitCode::from(2);
        }
    }

    // Register `loop://` BEFORE initialize, so the protocol exists by the time
    // the play command is issued.
    //
    // `user_data` is a LEAKED Box, not a pointer to a local. mpv keeps this
    // pointer until mpv_terminate_destroy returns and may dereference it from
    // its own threads at any point; aiming it at a stack slot in main() worked
    // only because every exit path happens to tear mpv down first. That is UB
    // the moment anything unwinds (a dev build does), and one refactor away
    // from UB even in release. 16 bytes, leaked once, removes the hazard.
    let proto = CString::new("loop").unwrap();
    let user_data = Box::into_raw(Box::new(leaked)) as *mut c_void;
    let rc = unsafe { mpv_stream_cb_add_ro(ctx, proto.as_ptr(), user_data, Some(open_fn)) };
    if rc < 0 {
        eprintln!("error: {}", err(ctx, "mpv_stream_cb_add_ro", rc));
        unsafe { mpv_terminate_destroy(ctx) };
        return ExitCode::FAILURE;
    }

    // Without this, libmpv discards every diagnostic it produces: `terminal=no`
    // is the libmpv default, so log output goes nowhere unless it is requested
    // as events. For an appliance whose value is a performance property that
    // functional testing cannot see, this is the difference between a field
    // failure being diagnosable and being a mystery.
    let lvl = CString::new("warn").unwrap();
    let rc = unsafe { mpv_request_log_messages(ctx, lvl.as_ptr()) };
    if rc < 0 {
        eprintln!("warning: {}", err(ctx, "mpv_request_log_messages", rc));
    }

    let rc = unsafe { mpv_initialize(ctx) };
    if rc < 0 {
        eprintln!("error: {}", err(ctx, "mpv_initialize", rc));
        unsafe { mpv_terminate_destroy(ctx) };
        return ExitCode::FAILURE;
    }

    // F1 tier-0 health check: subscribe to time-pos so the event loop can
    // later detect a stalled decode without ever polling the core
    // synchronously -- see dex_loop::health's module doc for the full
    // reasoning. This call is itself documented non-blocking; only the
    // SUBSEQUENT samples, delivered as ordinary MPV_EVENT_PROPERTY_CHANGE
    // events through the same wait loop already proven (by END_FILE and
    // QUEUE_OVERFLOW handling) never to hang, are load-bearing. A failure
    // to register is NOT fatal -- the health check is a best-effort safety
    // net on top of a working player, not a gate the show depends on -- but
    // per principle 2 it must be loud, never silent.
    let health_check_registered = observe(ctx, HEALTH_CHECK_USERDATA, "time-pos", MPV_FORMAT_DOUBLE);
    // Whether the health check is actually usable this run. Gates the tick
    // loop below (`health: Option<HealthMonitor>`) -- without this gate, a
    // failed registration would leave `last_position` permanently `None`,
    // and every tick would read as a stall forever, eventually issuing
    // recovery commands (and, once the budget is exhausted, exiting)
    // against a perfectly healthy player: the exact opposite of the
    // "DISABLED" warning below. `observe()` already logged the underlying
    // mpv error; this is the feature-specific consequence of that failure.
    if !health_check_registered {
        eprintln!(
            "warning: tier-0 self-healing is DISABLED for this run (time-pos \
             subscription failed above); tier 1 (process restart on a fatal \
             event) still applies"
        );
    }

    // F9: the heartbeat's two drop counters, subscribed the same way as
    // time-pos above and for the same reason -- see dex_loop::heartbeat's
    // module doc. Registered here, before `loadfile`, simply to group all of
    // this program's mpv_observe_property calls at startup rather than
    // scattering them -- registration order relative to loadfile does not
    // affect completeness either way: mpv forces an initial notification for
    // EVERY observer at the moment it registers (client.h), regardless of
    // when that happens, so nothing is "missed" by a later subscription.
    // That forced initial event for these two properties arrives promptly
    // as format=NONE/data=NULL (no VO chain yet -- see the property-change
    // handler below); the first real INT64 value is a second, later event
    // once the VO chain exists. A failed registration is not fatal --
    // `observe()` already warned loudly -- it just means this run's
    // heartbeat prints "off" for that counter for its whole life; no
    // separate warning is needed beyond `observe`'s own, since the
    // heartbeat repeats the fact every 10 minutes anyway.
    let mut frame_drops = if observe(ctx, FRAME_DROPS_USERDATA, "frame-drop-count", MPV_FORMAT_INT64) {
        ObservedCounter::observed()
    } else {
        ObservedCounter::unobserved()
    };
    let mut vo_delayed = if observe(ctx, VO_DELAYED_USERDATA, "vo-delayed-frame-count", MPV_FORMAT_INT64) {
        ObservedCounter::observed()
    } else {
        ObservedCounter::unobserved()
    };

    let cmd_loadfile = CString::new("loadfile").unwrap();
    let cmd_url = CString::new("loop://endless").unwrap();
    let argv: [*const c_char; 3] = [cmd_loadfile.as_ptr(), cmd_url.as_ptr(), std::ptr::null()];
    let rc = unsafe { mpv_command(ctx, argv.as_ptr()) };
    if rc < 0 {
        eprintln!("error: {}", err(ctx, "loadfile", rc));
        unsafe { mpv_terminate_destroy(ctx) };
        return ExitCode::FAILURE;
    }

    // Run until mpv stops playing. The stream is infinite by construction, so
    // there is no benign way for playback to end: END_FILE means something
    // failed, and it must be FATAL here.
    //
    // This is the single most important behaviour in the program, and it is not
    // obvious. libmpv is not the CLI player: `mpv_create` enables idle mode by
    // default (client.h), so a failed playback emits END_FILE and then sits in
    // idle FOREVER -- it never emits SHUTDOWN. A loop that waits only for
    // SHUTDOWN therefore blocks forever with the process alive, healthy to any
    // supervisor, and the wall black. That is strictly worse than crashing.
    //
    // Compounding it: with vo=gpu the video output is created during file load,
    // NOT during mpv_initialize. So every display-side failure -- projector not
    // awake, no EDID, DRM master held by a getty -- passes both the initialize
    // and loadfile return codes and lands here. Which is exactly the most
    // likely failure in a gallery.
    //
    // So: exit non-zero and let the supervisor restart us.
    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    // F1 tier-0 health check state. `last_position` is updated ONLY by the
    // MPV_EVENT_PROPERTY_CHANGE handler below -- never read synchronously
    // from mpv -- and fed to `health` on a fixed cadence. See
    // dex_loop::health's module doc for the full policy and reasoning. The
    // Instant travels WITH the position (one tuple, not two separately
    // updated locals) so they cannot desync -- see dex_loop::heartbeat's
    // `PositionSample` doc for why that is a struct-shape decision, not
    // just a style one.
    let mut last_position: Option<(f64, Instant)> = None;
    // F10: resolve the systemd watchdog handshake and open its socket (if
    // armed) BEFORE heartbeat #0, so that very first line already reports
    // the real watchdog state instead of a stale default -- see
    // dex_loop::watchdog's module doc and `setup_watchdog` above.
    let mut watchdog_runtime = setup_watchdog(HEALTH_CHECK_SECS);
    // Heartbeat #0: proves temperature reading and line formatting on every
    // boot, and anchors the journal. It is NOT proof of the mpv property
    // subscriptions (F9 removed the only call that could prove that
    // synchronously): frame-drops/vo-delayed/pos all read "n/a" here by
    // design, since nothing has been decoded yet. The on-device check that
    // the subscriptions actually work belongs to the deploy checklist, not
    // this line -- see PLAN.md's F9 entry ("must show numbers, not n/a").
    emit_heartbeat(
        started,
        last_position,
        frame_drops,
        vo_delayed,
        watchdog_runtime.as_ref().map(|w| w.pings_dropped),
    );
    let mut health: Option<HealthMonitor> = if health_check_registered {
        Some(HealthMonitor::new(MAX_RECOVERY_ATTEMPTS))
    } else {
        None
    };
    let mut last_health_check = Instant::now();
    // T7 (PLAN.md): armed only when the CLI gate above accepted
    // --force-recovery-after-secs (which itself required --bench-no-sidecar).
    // `None` here is the overwhelmingly common case -- every real deployment
    // run -- and costs one `Option` check per loop iteration.
    let mut force_recovery_trigger: Option<ForceRecoveryTrigger> =
        force_recovery_after_secs.map(ForceRecoveryTrigger::new);
    // F10 (PLAN.md): the bench-only wedge probe, armed only when the CLI
    // gate below accepted --bench-wedge-after-secs (which itself requires
    // --bench-no-sidecar, same escape hatch as T7). See its firing site
    // below for what it proves and why.
    let mut bench_wedge_trigger: Option<ForceRecoveryTrigger> =
        bench_wedge_after_secs.map(ForceRecoveryTrigger::new);
    // Counts recoveries issued whose matching END_FILE(reason=stop) has not
    // yet been observed (bench-confirmed live, three independent reviews,
    // 2026-08-15) -- see the MPV_EVENT_END_FILE handler below and PLAN.md's
    // F1 addendum. A COUNT, not a single flag: the organic tick and T7's
    // forced probe can both issue a recovery in the same loop iteration
    // (mpv_command_async only queues against a core that may still be busy
    // from a prior attempt), so more than one can be in flight at once, and
    // each produces its own END_FILE(stop) to absorb. Nothing else in this
    // program ever issues a command that produces a STOP-reason end-file, so
    // "count > 0" is what tells "one of our own recoveries' expected
    // teardowns" apart from an actual failure that happens to carry the same
    // reason code.
    let mut recovery_stops_pending: u32 = 0;

    let exit = ExitCode::SUCCESS;
    loop {
        // Wake at least every HEALTH_CHECK_SECS. In the healthy steady
        // state mpv delivers frequent time-pos property-change events on
        // its own, waking this thread without any help from the timeout --
        // but during an actual STALL, by definition NO such events arrive
        // (that absence IS the stall signal; see dex_loop::health), so the
        // timeout is what guarantees the health check still gets evaluated
        // on schedule in precisely the one case that matters. A wake this
        // cheap (drain one event, compare two numbers) on the event thread,
        // separate from the decode/VO threads, costs nothing on the decode
        // path.
        let ev = unsafe { mpv_wait_event(ctx, HEALTH_CHECK_SECS as f64) };
        let id = unsafe { (*ev).event_id };
        let reply_userdata = unsafe { (*ev).reply_userdata };

        if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_SECS {
            emit_heartbeat(
                started,
                last_position,
                frame_drops,
                vo_delayed,
                watchdog_runtime.as_ref().map(|w| w.pings_dropped),
            );
            last_heartbeat = Instant::now();
        }

        // F1's tick AND F10's ping share this cadence gate, but the ping is
        // NOT nested inside `if let Some(h) = health` below -- see
        // dex_loop::watchdog's module doc, "gate placement": if the
        // time-pos subscription itself failed to register (health is
        // `None`, near-zero probability), F1 is disabled but the player may
        // still be perfectly healthy, and stopping pings in that mode would
        // convert a merely-degraded run into a guaranteed watchdog kill
        // loop. The ping is emitted AFTER the tick/act_on_health_action
        // pair for this iteration has fully completed, per PLAN.md F10 §1 --
        // that ordering, plus F1's cumulative never-refilling recovery
        // budget (dex_loop::health, "why the budget never resets"), is what
        // makes this a real liveness criterion rather than "the process
        // runs": a display-wedged player's tick sequence is FORCED, by
        // construction, through silence -> stall -> <=3 budgeted recoveries
        // -> Escalate -> process exit, so it can only ever emit a BOUNDED
        // number of pings before either exiting (tier 1 already handles
        // that) or -- on the one path that can still hang, e.g. `eprintln!`
        // against a wedged journald on the Escalate arm itself, see that
        // arm's own comment below -- simply stopping, which is exactly what
        // the watchdog is here to catch. The loop cannot both hang and keep
        // pinging. (Scope: that covers wedged-core/wedged-thread failures
        // only -- a signal-level failure where time-pos advances with no
        // photons on the wall pings forever, out of F10's scope by design;
        // see dex_loop::watchdog's module doc, "Scope, stated precisely".)
        if last_health_check.elapsed().as_secs() >= HEALTH_CHECK_SECS {
            last_health_check = Instant::now();
            if let Some(h) = health.as_mut() {
                let action = h.tick(last_position.map(|(secs, _)| secs));
                act_on_health_action(
                    ctx,
                    action,
                    &format!(
                        "no progress across 2 consecutive checks ({HEALTH_CHECK_SECS}s apart)"
                    ),
                    &mut last_position,
                    &mut recovery_stops_pending,
                );
            }
            if let Some(wd) = watchdog_runtime.as_mut() {
                match watchdog::send_ping(&wd.socket, &wd.addr) {
                    PingOutcome::Sent => {}
                    PingOutcome::Dropped => wd.pings_dropped = wd.pings_dropped.saturating_add(1),
                }
            }
        }

        // T7 (PLAN.md): the bench-only live-fire probe. Checked every
        // iteration (two integer comparisons; `ForceRecoveryTrigger` is
        // `None` and this whole block skipped on every real deployment run)
        // rather than gated on the HEALTH_CHECK_SECS cadence above, so it
        // does not additionally wait out however much of that ~10s window
        // was already elapsed when `--force-recovery-after-secs` was
        // reached. It can still be delayed up to HEALTH_CHECK_SECS in the
        // worst case (mpv_wait_event's timeout bounds how often this loop
        // body runs at all when nothing else is waking it) -- acceptable
        // for a bench diagnostic whose job is to prove the mechanism works
        // at all, not to fire at a precise instant.
        if let Some(trigger) = force_recovery_trigger.as_mut() {
            if trigger.should_fire(started.elapsed().as_secs()) {
                match health.as_mut() {
                    Some(h) => {
                        let action = h.force_recovery();
                        act_on_health_action(
                            ctx,
                            action,
                            "T7 bench probe: --force-recovery-after-secs elapsed",
                            &mut last_position,
                            &mut recovery_stops_pending,
                        );
                    }
                    None => {
                        // health_check_registered was false (the time-pos
                        // subscription itself failed at startup, already
                        // warned loudly there) -- there is no HealthMonitor
                        // to spend a recovery attempt from, so the probe is
                        // inert. Loud, not silent: a bench operator staring
                        // at a run that never fires needs to know why.
                        eprintln!(
                            "warning: --force-recovery-after-secs elapsed but cannot fire: \
                             tier-0 health check is DISABLED for this run (time-pos \
                             subscription failed above)"
                        );
                    }
                }
            }
        }

        // F10: the bench-only wedge probe -- deliberately parks THIS event
        // thread forever, simulating the one hazard class F1/F9 cannot see
        // (a hang in our own code that is not an mpv call at all -- see
        // dex_loop::watchdog's module doc "Framing"). Checked every
        // iteration, same reasoning as T7's trigger above, and placed
        // BEFORE the event-id dispatch below for the same reason that
        // matters here even more than it does for T7: on a run where mpv
        // reaches END_FILE/QUEUE_OVERFLOW almost immediately (e.g. this
        // file's own `--opt vid=no --opt aid=no` test convention), the
        // dispatch's own `std::process::exit(1)` would otherwise win the
        // race and this probe would never get a chance to fire at all.
        // Firing hangs the thread PERMANENTLY (a real `loop`, not a single
        // long sleep) -- there is no "and then it resumes"; the whole point
        // is that only an external actor (systemd's watchdog, if armed; the
        // test harness's own deadline-kill otherwise) can end this process
        // from here on. If nothing ever un-hangs it, that IS the pass
        // condition -- see PLAN.md's F10 entry and README.md for how this
        // is used on the Pi to prove the watchdog fires.
        if let Some(trigger) = bench_wedge_trigger.as_mut() {
            if trigger.should_fire(started.elapsed().as_secs()) {
                eprintln!(
                    "warning: BENCH ONLY (F10 wedge probe): --bench-wedge-after-secs elapsed -- \
                     deliberately parking the event thread forever to simulate F10's hazard \
                     class (an event-thread hang outside any mpv call). If a systemd watchdog \
                     is armed above, it should fire and this process should be restarted by \
                     the supervisor; if this process is still alive well past WatchdogSec, the \
                     watchdog did not fire and F10 has a gap."
                );
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
        }

        if id == MPV_EVENT_NONE {
            continue;
        }
        if id == MPV_EVENT_SHUTDOWN {
            break;
        }
        if id == MPV_EVENT_START_FILE {
            continue;
        }
        if id == MPV_EVENT_PROPERTY_CHANGE {
            // SAFETY: `data` is an mpv_event_property for this event id,
            // valid until the next mpv_wait_event call.
            let p = unsafe { &*((*ev).data as *const MpvEventProperty) };
            // Check BOTH the reply_userdata tag and the format tag before
            // ever touching `data` -- `data`'s true type is decided by
            // `format` (a tagged union), and trusting a hand-transcribed
            // format constant without checking it is exactly the class of
            // bug that made MPV_EVENT_LOG_MESSAGE's mistranscription a
            // segfault instead of a caught error. A mismatch on either tag
            // is not acted on: the next health-check tick simply sees no
            // new sample, which HealthMonitor already treats identically to
            // a genuine stall (see dex_loop::health's module doc) -- so
            // failing to interpret an unexpected payload here fails toward
            // "the health check is slightly more eager", never toward
            // reading garbage. F9's two drop-counter observers follow the
            // exact same discipline for MPV_FORMAT_INT64 -- and DO also act
            // on MPV_FORMAT_NONE, deliberately, see below.
            if reply_userdata == HEALTH_CHECK_USERDATA
                && p.format == MPV_FORMAT_DOUBLE
                && !p.data.is_null()
            {
                // SAFETY: the format tag confirms `data` points to an f64,
                // per client.h's mpv_event_property contract for
                // MPV_FORMAT_DOUBLE. `.cast::<f64>()` rather than
                // `as *const f64`: this crate's convention (see read_fn's
                // SAFETY comment above, where a platform-dependent `as`
                // cast already caused a real bug) is `.cast()` for every raw
                // pointer conversion, so a pointee-type mismatch is always a
                // compile error instead of a silent reinterpretation.
                last_position = Some((unsafe { *p.data.cast::<f64>() }, Instant::now()));
            } else if p.format == MPV_FORMAT_INT64 && !p.data.is_null() {
                // SAFETY: the format tag confirms `data` points to an i64,
                // per MPV_FORMAT_INT64's doc comment in ffi_consts.rs.
                let raw = unsafe { *p.data.cast::<i64>() };
                match reply_userdata {
                    FRAME_DROPS_USERDATA => frame_drops.sample(raw),
                    VO_DELAYED_USERDATA => vo_delayed.sample(raw),
                    _ => {}
                }
            } else if p.format == MPV_FORMAT_NONE
                && matches!(reply_userdata, FRAME_DROPS_USERDATA | VO_DELAYED_USERDATA)
            {
                // A property that is momentarily UNAVAILABLE (no vo_chain:
                // before the first frame, and during a recovery's teardown)
                // arrives as format=MPV_FORMAT_NONE with data=NULL
                // (player/client.c:1810-1816). `total` must survive this
                // untouched -- it is not a value and not itself a counter
                // reset -- but `last_raw` must NOT: mpv coalesces property
                // events (client.h: "only once the event queue becomes
                // empty ... one event per changed property"), so if a
                // recovery's teardown (unavailable), the new session's
                // restart at 0, and a climb past the old session's total all
                // happen before this event thread next drains -- plausible
                // exactly then, since the thread is busy absorbing the
                // recovery's END_FILE/START_FILE burst -- `sample()` would
                // see e.g. 5 -> 7 with no visible decrease and under-count
                // by however many drops actually occurred (MINOR, three
                // adversarial reviews, 2026-08-15). Recording the
                // unavailability here means the next delivered sample,
                // however small, is read as a fresh first sample rather
                // than diffed against a `last_raw` that may already belong
                // to a dead session -- narrows the window (this NONE event
                // itself could still be coalesced away) rather than closing
                // it; full closure isn't possible from the client side and
                // isn't worth more machinery for a diagnostic line. See
                // `ObservedCounter::mark_unavailable`'s doc comment.
                match reply_userdata {
                    FRAME_DROPS_USERDATA => frame_drops.mark_unavailable(),
                    VO_DELAYED_USERDATA => vo_delayed.mark_unavailable(),
                    _ => {}
                }
            }
            continue;
        }
        if id == MPV_EVENT_COMMAND_REPLY {
            // Diagnostic only -- nothing gates on this. The next
            // health-check tick judges the recovery by its actual effect
            // (did time-pos start advancing again), not by whether mpv
            // accepted the command; logging a rejection just makes that
            // judgment call diagnosable from the journal afterwards.
            if reply_userdata == RECOVERY_COMMAND_USERDATA {
                let error = unsafe { (*ev).error };
                if error < 0 {
                    eprintln!(
                        "dex-loop: health check: in-place recovery's loadfile command \
                         was rejected: {}",
                        err(ctx, "mpv_command_async(loadfile) reply", error)
                    );
                    // Rejected AFTER being queued (mpv_command_async itself
                    // returned success) but before taking effect: no
                    // matching END_FILE(reason=stop) will ever arrive for
                    // THIS attempt, so its slot must not sit waiting for one
                    // -- a stale count left too high here could otherwise
                    // mask a real, later, unrelated END_FILE(stop) as an
                    // attempt's expected teardown. Decrement by one, not
                    // reset to zero: RECOVERY_COMMAND_USERDATA is shared by
                    // every recovery command, so a second attempt may
                    // legitimately still be in flight and its own stop is
                    // still owed.
                    recovery_stops_pending = recovery_stops_pending.saturating_sub(1);
                }
            }
            continue;
        }
        if id == MPV_EVENT_LOG_MESSAGE {
            // SAFETY: mpv guarantees `data` is an mpv_event_log_message for
            // this event id, with NUL-terminated strings valid until the next
            // mpv_wait_event call.
            let m = unsafe { &*((*ev).data as *const MpvEventLogMessage) };
            let pfx = unsafe { CStr::from_ptr(m.prefix) }.to_string_lossy();
            let txt = unsafe { CStr::from_ptr(m.text) }.to_string_lossy();
            eprint!("mpv/{pfx}: {txt}");
            continue;
        }
        if id == MPV_EVENT_END_FILE {
            // SAFETY: `data` is an mpv_event_end_file for this event id.
            let ef = unsafe { &*((*ev).data as *const MpvEventEndFile) };
            if is_expected_recovery_stop(recovery_stops_pending, ef.reason) {
                // One of the in-place recovery's `loadfile ... replace`
                // calls just produced the END_FILE(reason=stop) mpv always
                // emits for the file being replaced -- the expected teardown
                // half of a recovery that is still in progress, not a
                // failure. See MPV_END_FILE_REASON_STOP's doc comment and
                // PLAN.md's F1 addendum: before this check existed, EVERY
                // in-place recovery attempt killed the process on its own
                // first step, making tier 0 unreachable. Absorb exactly one
                // -- not reset to zero -- because the organic tick and T7's
                // forced probe can both have issued a recovery in the same
                // loop iteration, in which case a SECOND matching stop is
                // still owed and must not be treated as fatal either.
                recovery_stops_pending -= 1;
                eprintln!(
                    "dex-loop: health check: in-place recovery's loadfile replaced the \
                     stream; absorbing the expected END_FILE(reason=stop) for the file \
                     it replaced ({recovery_stops_pending} more still outstanding), not \
                     treating it as a failure"
                );
                continue;
            }
            let why = unsafe { CStr::from_ptr(mpv_error_string(ef.error)) };
            eprintln!(
                "dex-loop: FATAL: playback ended (reason={}, error={}) -- an endless \
                 stream must never end; exiting so the supervisor restarts",
                ef.reason,
                why.to_string_lossy()
            );
            // std::process::exit, not `break`: see the Escalate arm's
            // comment above for why the shared mpv_terminate_destroy()
            // teardown is the wrong tool on a fatal exit path -- the same
            // reasoning applies here (and to QUEUE_OVERFLOW below).
            std::process::exit(1);
        }
        if id == MPV_EVENT_QUEUE_OVERFLOW {
            // mpv's internal event ring chokes at 1000 pending events and
            // silently drops every event after that -- including END_FILE --
            // until the client drains back to empty (client.c send_event).
            // There is no reservation for fatal events, so a dropped
            // END_FILE would otherwise leave this program in mpv's default
            // idle mode forever: exactly bug #1's failure, entered through a
            // different door. We cannot know what was lost, so treat this
            // exactly like END_FILE: exit and let the supervisor restart.
            eprintln!(
                "dex-loop: FATAL: mpv event queue overflowed -- at least one event was \
                 dropped and may have been the one that mattered; exiting so the \
                 supervisor restarts"
            );
            std::process::exit(1);
        }
    }

    unsafe { mpv_terminate_destroy(ctx) };
    exit
}

// ---------------------------------------------------------------------------
// This is the `dex-loop` BIN target, so these tests link libmpv (Pi only:
// `cargo test`) even though they call no mpv function -- `cargo check
// --all-targets` type-checks them on the Mac without linking. `cargo test
// --lib` (the Mac-safe command) does not run this module; it only runs
// tests under the `dex_loop` LIB target (src/lib.rs and its submodules).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // Historical bug #3 lived exactly at this line: `read_fn` returning 0
    // for a zero-length request, which mpv reads as final EOF. `chunk.rs`'s
    // `next_chunk(_, _, 0) == None` test pins the pure boundary; this test
    // pins the shell around it -- reintroduce `else { return 0; }` here and
    // every test in the crate stays green except this one (mpv itself
    // essentially never issues a zero-length read, so the Pi integration
    // suite can't see it either).
    #[test]
    fn read_fn_reports_mpv_error_not_zero_for_a_zero_length_request() {
        let data: &'static [u8] = Box::leak(vec![1u8, 2, 3].into_boxed_slice());
        let cookie = Box::into_raw(Box::new(LoopStream { data, pos: 0 })) as *mut c_void;
        let mut buf = [0u8; 8];
        let r = read_fn(cookie, buf.as_mut_ptr().cast::<c_char>(), 0);
        assert_eq!(
            r,
            i64::from(MPV_ERROR_UNSUPPORTED),
            "must be an mpv error, never 0 -- to mpv, 0 means final EOF"
        );
        // Reclaim what open_fn would normally leave leaked for the stream's
        // lifetime, via the same path close_fn uses.
        unsafe { drop(Box::from_raw(cookie as *mut LoopStream)) };
    }

    #[test]
    fn read_fn_copies_bytes_and_advances_the_shared_position() {
        let data: &'static [u8] = Box::leak(vec![10u8, 20, 30, 40, 50].into_boxed_slice());
        let cookie = Box::into_raw(Box::new(LoopStream { data, pos: 0 })) as *mut c_void;
        let mut buf = [0u8; 8];

        let r = read_fn(cookie, buf.as_mut_ptr().cast::<c_char>(), 3);
        assert_eq!(r, 3);
        assert_eq!(&buf[..3], &[10, 20, 30]);
        // SAFETY: single-threaded test; no other call is touching `cookie`.
        let pos_after_first = unsafe { &*(cookie as *mut LoopStream) }.pos;
        assert_eq!(pos_after_first, 3, "the callback must thread position through the same cookie, not reset per call");

        let r = read_fn(cookie, buf.as_mut_ptr().cast::<c_char>(), 4);
        assert_eq!(r, 2, "short read: only 2 bytes remain before the wrap");
        assert_eq!(&buf[..2], &[40, 50]);
        let pos_after_second = unsafe { &*(cookie as *mut LoopStream) }.pos;
        assert_eq!(pos_after_second, 0, "eager wrap: position must land back at 0, not at len");

        unsafe { drop(Box::from_raw(cookie as *mut LoopStream)) };
    }

    // The C1 regression this pins: `loadfile ... replace` (F1's in-place
    // recovery) makes mpv emit END_FILE(reason=stop) for the file being
    // replaced. Before `is_expected_recovery_stop` existed, the event loop
    // treated ANY end-file as fatal, so the recovery's own first step
    // always killed the process -- tier 0 was unreachable through the real
    // event loop. These three cases are the whole fix.

    #[test]
    fn a_pending_recovery_absorbs_its_own_stop_reason_end_file() {
        assert!(is_expected_recovery_stop(1, MPV_END_FILE_REASON_STOP));
    }

    #[test]
    fn a_stop_reason_end_file_with_no_recovery_pending_stays_fatal() {
        // Nothing else in this program issues a command that produces a
        // stop-reason end-file -- but if one ever did, it must not be
        // silently swallowed just because the reason code matches.
        assert!(!is_expected_recovery_stop(0, MPV_END_FILE_REASON_STOP));
    }

    #[test]
    fn any_other_reason_stays_fatal_even_while_a_recovery_is_pending() {
        // eof=0, quit=3, error=4, redirect=5 (2 is stop, tested above).
        for reason in [0, 3, 4, 5] {
            assert!(
                !is_expected_recovery_stop(1, reason),
                "reason {reason} must not be absorbed"
            );
        }
    }

    // The follow-up regression these two pin (three independent adversarial
    // reviews, 2026-08-15, MAJOR): the organic health-check tick and T7's
    // forced probe can both issue a recovery before either one's END_FILE
    // arrives (mpv_command_async only QUEUES against a core that may still
    // be busy from the first attempt), producing TWO END_FILE(reason=stop)
    // events for one episode. A single bool could absorb only the first and
    // treated the second -- an entirely expected teardown for a recovery
    // that just worked -- as fatal, killing a healthy-again process with a
    // journal line indistinguishable from C1. See main.rs's
    // `recovery_stops_pending` doc comment and PLAN.md's F1 addendum.

    #[test]
    fn two_overlapping_recoveries_both_absorb_their_own_stop() {
        let mut pending: u32 = 0;
        pending += 1; // organic tick issues attempt 1
        pending += 1; // T7's forced probe issues attempt 2, same iteration
        assert_eq!(pending, 2);

        assert!(is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP));
        pending -= 1; // first END_FILE(stop) absorbed
        assert_eq!(pending, 1, "one recovery's stop is still owed");

        assert!(
            is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP),
            "the second stop must NOT be treated as fatal just because the \
             first one already cleared the flag -- this is the exact bug a \
             plain bool could not represent"
        );
        pending -= 1;
        assert_eq!(pending, 0);

        // Budget is exhausted now (both attempts absorbed): a THIRD
        // stop-reason end-file with nothing outstanding is a real failure.
        assert!(!is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP));
    }

    #[test]
    fn a_rejected_command_reply_decrements_by_one_not_to_zero() {
        // Mirrors the MPV_EVENT_COMMAND_REPLY handler: a rejection after
        // queueing means THAT attempt's stop will never arrive, but
        // RECOVERY_COMMAND_USERDATA is shared by every recovery command, so
        // a second, still-legitimate attempt may be in flight and its stop
        // must remain expected.
        let mut pending: u32 = 2;
        pending = pending.saturating_sub(1); // one attempt's reply was rejected
        assert_eq!(pending, 1);
        assert!(is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP));
    }
}
