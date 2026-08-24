//! Gapless HEVC video looper for the Raspberry Pi, built on libmpv.
//!
//! dexd registers a `loop://` stream whose read callback never reports end of
//! file: when the position reaches the end of the payload it wraps to byte 0,
//! so the decoder never re-enters the file and never seeks. The asset must be a
//! raw Annex-B HEVC elementary stream that begins with parameter sets and an
//! IDR, and its frame rate comes from the sidecar beside it, because a raw
//! stream carries no timestamps.
//!
//! See docs/design/endless-stream.md#the-endless-stream for the looping
//! mechanism and docs/design/architecture.md#architecture for how the process
//! is put together.

#![deny(unsafe_op_in_unsafe_fn)]

use dexd::chunk::{clamp_want, next_chunk};
use dexd::exhibit::{
    check_cmdline_matches, config_dir, load_exhibit_config, mode_resolution, resolve_asset,
    resolve_display, sysfs_modes_contains, AssetSource, DisplaySource, DEFAULT_EXHIBIT_CONFIG_PATH,
    DEFAULT_EXHIBIT_CONFIG_PATHS,
};
use dexd::ffi_consts::{
    MPV_END_FILE_REASON_STOP, MPV_ERROR_UNSUPPORTED, MPV_EVENT_COMMAND_REPLY, MPV_EVENT_END_FILE,
    MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_QUEUE_OVERFLOW,
    MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE, MPV_FORMAT_DOUBLE, MPV_FORMAT_INT64, MPV_FORMAT_NONE,
};
use dexd::health::{ForceRecoveryTrigger, HealthAction, HealthMonitor};
use dexd::heartbeat::{HeartbeatSnapshot, ObservedCounter, PositionSample};
use dexd::nal::validate_leading_nals;
use dexd::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar};
use dexd::watchdog::{self, PingOutcome, WatchdogDecision, WatchdogEnv};
use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs;
use std::os::unix::net::{SocketAddr as UnixSocketAddr, UnixDatagram};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ---------------------------------------------------------------------------
// libmpv FFI: the entry points this program uses, transcribed by hand from
// mpv/client.h and mpv/stream_cb.h. Keep the surface this small, and check
// every addition against the header it comes from.
// See docs/design/architecture.md#architecture.
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

/// `mpv_event_end_file`, truncated to the two fields this program reads.
/// Reading a prefix of a `#[repr(C)]` struct is well-defined, so the trailing
/// playlist fields of the C struct are left out.
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

/// `mpv_event_property`. `data` is a tagged union whose type is decided by
/// `format`, so check `format` before dereferencing `data` -- see
/// `MPV_FORMAT_DOUBLE` in `ffi_consts.rs`.
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
    // Queues the command and returns immediately (client.h), replying later
    // through MPV_EVENT_COMMAND_REPLY. In-place recovery uses this and never
    // the synchronous mpv_command, so the supervisor thread cannot block on it.
    // See docs/design/failure-handling.md#in-place-recovery.
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
    // Non-blocking (client.h): queues a subscription and returns immediately.
    // The health check registers once at startup and afterwards reads the
    // position only out of MPV_EVENT_PROPERTY_CHANGE events, never through a
    // synchronous property read.
    // See docs/design/failure-handling.md#health-check.
    fn mpv_observe_property(
        ctx: *mut MpvHandle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    // Not bound, and not to be added: mpv_get_property_string and
    // mpv_get_property, with the mpv_free they need. A synchronous property
    // read waits with no timeout for mpv's core thread to reach its dispatch
    // loop, and a core stuck in a DRM ioctl never reaches it, so the caller
    // blocks forever. Values come from mpv_observe_property and
    // MPV_EVENT_PROPERTY_CHANGE instead.
    // See docs/design/failure-handling.md#heartbeat.
}

// ---------------------------------------------------------------------------
// The endless stream
// ---------------------------------------------------------------------------

/// One reader's position within the looping payload.
///
/// The payload is `&'static [u8]`: it is leaked once at startup and outlives
/// every mpv thread, so no point in the run exists at which freeing it would be
/// correct. See docs/design/architecture.md#architecture.
struct LoopStream {
    data: &'static [u8],
    pos: usize,
}

// The cookie is created on one mpv thread, read on the demux thread, and freed
// on whichever thread closes the stream, so LoopStream must be Send. The raw
// pointer it travels through as `cookie` hides that from the compiler, so
// assert it here: a field that is not Send (an Rc, a raw pointer, an mmap
// guard) then fails to compile instead of racing.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<LoopStream>();
};

/// Completed passes over the payload: `read_fn` increments it on the demux
/// thread each time the position wraps to 0, and the heartbeat reads it on the
/// supervisor thread. It counts demuxer passes, which run about one second of
/// read-ahead ahead of the picture on screen. Relaxed ordering is enough for a
/// monotonic diagnostic counter.
static LOOP_COUNT: AtomicU64 = AtomicU64::new(0);

/// Heartbeat cadence, in seconds: often enough to place a failure inside a
/// ten-minute window of the system log, rare enough to cost nothing.
const HEARTBEAT_SECS: u64 = 600;

/// Health-check cadence, in seconds. The escalation policy
/// (`dexd::health::HealthMonitor`) needs two consecutive non-advancing checks
/// before it acts, so a stall is detected after roughly twice this.
/// See docs/design/failure-handling.md#health-check.
const HEALTH_CHECK_SECS: u64 = 10;

/// The recovery budget: how many in-place recoveries this process may attempt
/// in its lifetime. It never refills, so the worst case stays bounded; three is
/// enough to absorb isolated glitches such as an HDMI blink or a display waking
/// late over a multi-week unattended run.
/// See docs/design/failure-handling.md#recovery-budget.
const MAX_RECOVERY_ATTEMPTS: u32 = 3;

/// `reply_userdata` tag for the `time-pos` subscription, so its
/// MPV_EVENT_PROPERTY_CHANGE events stay distinct from any property this
/// program observes later.
const HEALTH_CHECK_USERDATA: u64 = 1;

/// `reply_userdata` tag for the in-place recovery's `loadfile` command, so its
/// MPV_EVENT_COMMAND_REPLY stays distinct from any async command this program
/// issues later.
const RECOVERY_COMMAND_USERDATA: u64 = 2;

/// `reply_userdata` tags for the two observed drop counters, distinct from each
/// other and from the tags above, so the MPV_EVENT_PROPERTY_CHANGE handler
/// never has to `CStr`-compare `p.name` to know which counter a payload belongs
/// to.
const FRAME_DROPS_USERDATA: u64 = 3;
const VO_DELAYED_USERDATA: u64 = 4;

/// The stream read callback: an unsafe shell over
/// [`dexd::chunk::next_chunk`], which implements and tests the three rules --
/// never return 0, wrap eagerly, saturate the u64 request size. This function
/// performs the copy the pure core cannot.
///
/// A zero-length request returns an mpv error and never 0, because to mpv 0 is
/// end of file, the one event this program exists to prevent. mpv 0.40 guards
/// `len <= 0` in `stream.c` before calling in, so the branch is a sentinel for
/// this crate's own diagnostics.
/// See docs/design/endless-stream.md#loop-stream.
extern "C" fn read_fn(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    // SAFETY: `cookie` is the Box<LoopStream> leaked in `open_fn`, and mpv
    // passes it back unmodified for the life of the stream. The `&mut` also
    // needs exclusivity, which comes from mpv's stream layer: a stream_t is
    // single-owner and driven by one thread at a time. `open_cb` runs the
    // seek_fn(cookie, 0) probe during open, before fill_buffer and close are
    // installed (stream/stream_cb.c), and close runs after demux teardown, so
    // open-time access happens before every read and close happens after the
    // last one. stream_cb.h promises none of this, and it documents cancel_fn
    // as cross-thread (stream_cb.h:154-155), so `cancel_fn` stays `None`;
    // wiring it up needs an atomic or a lock on the cookie first.
    let s = unsafe { &mut *(cookie as *mut LoopStream) };

    let Some(c) = next_chunk(s.data.len(), s.pos, clamp_want(nbytes)) else {
        // Zero-length request, or an empty payload: report an error and never
        // 0 -- see the doc comment above.
        return i64::from(MPV_ERROR_UNSUPPORTED);
    };

    // SAFETY: mpv guarantees `buf` is writable for `nbytes` bytes; next_chunk
    // guarantees 1 <= c.n <= nbytes (the request is clamped, never grown) and
    // c.start + c.n <= data.len(), and the ranges cannot overlap.
    //
    // `.cast::<u8>()` rather than `as *mut u8`: `c_char` is `i8` on
    // macOS/aarch64 and `u8` on Linux/aarch64, so `as` is a real conversion on
    // one machine and a cast clippy rejects on the other. Do not shorten it to
    // `buf` -- that compiles only on the Raspberry Pi.
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(c.start), buf.cast::<u8>(), c.n);
    }
    s.pos = c.next_pos;
    if c.next_pos == 0 {
        // The copy reached the payload's end: one full pass completed.
        LOOP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    c.n as i64
}

/// Report the stream as unseekable, like a pipe.
///
/// An mpv that can seek will seek, and a seek is what produces the pause at the
/// loop point. Refusing here leaves "keep reading forwards" as the only
/// available behaviour.
/// See docs/design/endless-stream.md#loop-point-stalls.
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

/// Open callback for the `loop://` protocol. The URL is ignored: the payload is
/// fixed at startup, so there is nothing to parse and nothing that can fail here.
extern "C" fn open_fn(
    user_data: *mut c_void,
    _uri: *mut c_char,
    info: *mut MpvStreamCbInfo,
) -> c_int {
    // SAFETY: `user_data` is the &'static [u8] passed to mpv_stream_cb_add_ro.
    let data: &'static [u8] = unsafe { *(user_data as *mut &'static [u8]) };
    let stream = Box::new(LoopStream { data, pos: 0 });

    // SAFETY: mpv provides a valid, writable info struct to fill in.
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

/// Register one property observer. `mpv_observe_property` only takes the
/// client-local lock and wakes the core (player/client.c:1536-1572), so this
/// never blocks. Returns whether the subscription is live for this run; a
/// failure is logged and never fatal.
fn observe(ctx: *mut MpvHandle, userdata: u64, name: &str, format: c_int) -> bool {
    let Ok(n) = CString::new(name) else {
        return false;
    };
    let rc = unsafe { mpv_observe_property(ctx, userdata, n.as_ptr(), format) };
    if rc < 0 {
        eprintln!(
            "warning: {}",
            err(ctx, &format!("mpv_observe_property({name})"), rc)
        );
    }
    rc >= 0
}

/// Print one heartbeat line to stderr, on the supervisor thread and off the
/// decode path. It takes no `*mut MpvHandle`, so it cannot call into mpv: every
/// value it prints was already learned from an event.
/// See docs/design/failure-handling.md#heartbeat.
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
            loops: LOOP_COUNT.load(Ordering::Relaxed),
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

/// The systemd watchdog resources held for the whole run: the non-blocking
/// socket (opened once, never recreated), the address it pings, and a running
/// count of dropped pings for the heartbeat line. `None` for every run outside
/// systemd (see [`setup_watchdog`]).
///
/// `dexd::watchdog` decides the parts that can be unit-tested on their own; the
/// socket and the `send_to_addr` call site stay here, with the rest of the
/// mpv-facing state.
struct WatchdogRuntime {
    socket: UnixDatagram,
    addr: UnixSocketAddr,
    pings_dropped: u64,
}

/// Decide what to do when the watchdog resolves as armed but its ping socket
/// cannot be set up (address resolution, socket creation, `set_nonblocking`).
///
/// - Systemd's kill timer running (`$WATCHDOG_USEC` present): exit(1) now.
///   Nothing this process logs disarms the timer, so running without pings
///   means being aborted every window, and the process exits so
///   `Restart=always` with `RestartSec=2` retries in seconds.
/// - No timer running: warn once and play the asset, since pings would prove
///   nothing.
///
/// See docs/design/failure-handling.md#ping-protocol.
fn watchdog_setup_failed(
    cause: &str,
    kill_timer_armed: bool,
    window_secs: Option<u64>,
) -> Option<WatchdogRuntime> {
    if kill_timer_armed {
        let window = window_secs
            .map(|w| format!("{w}s"))
            .unwrap_or_else(|| "WatchdogSec".to_string());
        eprintln!(
            "error: dexd: watchdog: {cause} -- systemd's WatchdogSec timer is armed \
             ($WATCHDOG_USEC is set) and cannot be disarmed from inside this process: without \
             pings, systemd would kill this process with SIGABRT every {window} while it plays \
             normally. Exiting now so Restart= retries cleanly instead."
        );
        std::process::exit(1);
    }
    eprintln!(
        "warning: dexd: watchdog: {cause} -- no WatchdogSec timer is armed ($WATCHDOG_USEC \
         absent), so pings would prove nothing; running without watchdog pings for this run"
    );
    None
}

/// Resolve the systemd watchdog handshake and, when it is armed, open the
/// non-blocking socket it needs. Failures along the way follow
/// [`watchdog_setup_failed`]'s policy: they downgrade to "no pings" only while
/// systemd's own kill timer is not running.
fn setup_watchdog(tick_secs: u64) -> Option<WatchdogRuntime> {
    let env = WatchdogEnv::from_process_env();
    // systemd exports $WATCHDOG_USEC only when WatchdogSec= is configured on
    // the unit, so its presence means a kill timer is already counting, whatever
    // this process does or logs about its own pings.
    let kill_timer_armed = env.watchdog_usec.is_some();
    let decision = watchdog::resolve(&env, std::process::id(), tick_secs);
    let (addr, window_secs, warning) = match decision {
        WatchdogDecision::Inert(reason) => {
            eprintln!("dexd: watchdog: inert ({reason})");
            // A pid mismatch is an ordinary inert condition while no timer is
            // armed. Under an armed WatchdogSec= it is not: systemd expects
            // pings from the unit's main pid, this process does not ping under
            // another pid, and the timer fires every window. Say so, so the
            // system log explains the kills that follow (for example after an
            // edit that wraps ExecStart in a shell).
            if kill_timer_armed
                && matches!(reason, watchdog::InertReason::WatchdogPidMismatch { .. })
            {
                eprintln!(
                    "warning: dexd: watchdog: $WATCHDOG_USEC is set, so systemd's WatchdogSec \
                     timer is armed and expects pings from the unit's main pid -- with none \
                     arriving, systemd will kill this process every watchdog window; expect a \
                     kill/restart loop until the unit is fixed (is ExecStart wrapped in a shell?)"
                );
            }
            return None;
        }
        WatchdogDecision::Armed {
            addr,
            window_secs,
            warning,
        } => (addr, window_secs, warning),
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
    // The socket must be non-blocking: a blocking send against a full receiver
    // queue would be another way to hang the supervisor thread.
    // See docs/design/failure-handling.md#ping-protocol.
    if let Err(e) = socket.set_nonblocking(true) {
        return watchdog_setup_failed(
            &format!(
                "set_nonblocking failed: {e} (refusing a socket that could block the \
                 supervisor thread)"
            ),
            kill_timer_armed,
            window_secs,
        );
    }

    let window = window_secs
        .map(|w| format!("{w}s"))
        .unwrap_or_else(|| "unknown".to_string());
    eprintln!("dexd: watchdog: armed (window {window}, ping cadence {tick_secs}s)");
    if let Some(w) = warning {
        eprintln!("warning: dexd: watchdog: {w}");
    }

    Some(WatchdogRuntime {
        socket,
        addr: sockaddr,
        pings_dropped: 0,
    })
}

/// Whether an `MPV_EVENT_END_FILE` with this `reason` is the expected teardown
/// half of an in-place recovery's own `loadfile ... replace`, and not a real
/// failure. See `MPV_END_FILE_REASON_STOP` for the mpv behaviour this encodes.
///
/// `recovery_stops_pending` is a count and not a flag: the health-check tick and
/// the forced-recovery probe can both issue a recovery in the same loop
/// iteration, and `mpv_command_async` only queues a loadfile against a core that
/// may still be busy, so two recoveries can be in flight and each produces its
/// own end-file event to absorb.
/// See docs/design/failure-handling.md#expected-end-of-file.
fn is_expected_recovery_stop(recovery_stops_pending: u32, reason: c_int) -> bool {
    recovery_stops_pending > 0 && reason == MPV_END_FILE_REASON_STOP
}

/// Act on a [`HealthAction`], whatever produced it. The health-check tick and
/// the `--test-rig-force-recovery-after-secs` probe
/// (`HealthMonitor::force_recovery`) share this function, so a forced probe
/// drives the same mpv-facing steps a real stall would:
/// `mpv_command_async(loadfile ... replace)`, then incrementing
/// `recovery_stops_pending` so the resulting `END_FILE(reason=stop)` is
/// absorbed (see `is_expected_recovery_stop`). `reason` is the log prefix only.
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
                "dexd: health check: {reason} -- attempting in-place recovery \
                 {attempt}/{max} (re-issuing loadfile: restarts demux and decode and forces \
                 a video-output reconfigure; does not tear down or re-initialise the \
                 DRM/GPU context itself)"
            );
            // Mirror HealthMonitor's own baseline reset (health.rs sets
            // `self.last_position = None`). Without it the driver would feed
            // the pre-recovery position into the next tick, and the monitor,
            // comparing against its own `None` baseline, would read that as
            // fresh progress and grant a healthy tick that stretches the
            // escalation timeline. The sample's `Instant` travels in the same
            // tuple, so this one line also clears the age shown as `pos-age=`.
            *last_position = None;
            // mpv_command_async, not mpv_command: this call runs on the same
            // supervisor thread that must keep detecting every fatal event,
            // and a synchronous command could block that thread against an
            // unresponsive core.
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
                // The command was queued, so mpv will deliver an
                // END_FILE(reason=stop) for the file being replaced (see
                // MPV_END_FILE_REASON_STOP), and that event must be absorbed.
                // When mpv_command_async itself failed above, no such event is
                // coming and the count must stay as it is. Saturating: the
                // recovery budget bounds this in practice, so saturation never
                // engages; it is here so a later change to that relationship
                // fails safe, as an undercount that stays fatal.
                *recovery_stops_pending = recovery_stops_pending.saturating_add(1);
            }
        }
        HealthAction::Escalate => {
            eprintln!(
                "dexd: fatal: in-place recovery exhausted its budget \
                 ({MAX_RECOVERY_ATTEMPTS} attempt(s)) with no progress ({reason}) -- \
                 exiting so systemd restarts this process"
            );
            // std::process::exit, not `break` into the shared
            // mpv_terminate_destroy() teardown at the end of main:
            // mpv_terminate_destroy joins mpv's own threads, and a core that
            // cannot advance time-pos may not complete that join, which would
            // block the one exit path whose job is to hand the player over to
            // systemd.
            //
            // The `eprintln!` above can itself block, for example against an
            // unresponsive system log, and that is what the systemd watchdog
            // (`WatchdogSec=` in deploy/dexd.service, `dexd::watchdog`) exists
            // to catch: if this arm hangs, the loop completes no further
            // iteration, no more pings are sent, and systemd's timer fires. Do
            // not add a final ping here -- it would reset that countdown right
            // before the hang the watchdog exists to catch.
            // See docs/design/failure-handling.md#ping-protocol.
            std::process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: dexd [<stream.265>] [--fps <F>] [--mode WxH@R] [--test-rig-no-sidecar] [--no-defaults] [--opt K=V ...]

  <stream.265>        raw Annex-B HEVC elementary stream, looped endlessly.
                      Optional in a deployment: the exhibit config's `asset` key
                      names it, which is what lets several assets sit in
                      /opt/dex with the exhibit choosing one. A relative `asset`
                      resolves against the directory the config file is in.
                      Given here too, it must agree with the config's resolved
                      path or startup refuses, naming both. Required with
                      --test-rig-no-sidecar, which consults no config. If
                      neither names an asset, startup refuses rather than
                      guessing an artwork.
  <stream.265>.json   sidecar, required: {{\"fps\":\"30\",\"sha256\":\"<64 hex>\"}}
                      fps comes from it; the sha256 must match the asset bytes
  --fps F             optional cross-check; must equal the sidecar fps
  --mode WxH@R        cross-check against the exhibit config's display_mode;
                      optional alongside --test-rig-no-sidecar, where it is the only
                      source instead (defaults to auto there)
  --exhibit-config PATH
                      path to the exhibit config. Default: whichever of
                      {DEFAULT_EXHIBIT_CONFIG_PATHS:?} exists -- exactly one may,
                      and two at once is refused rather than resolved by
                      precedence. The file extension selects the parser: .json is
                      strict JSON, .yaml/.yml is YAML; same schema either way.
                      Binds the display mode, the expected forced mode, and the
                      connector -- see man dex-exhibit-apply.
  --test-rig-no-sidecar
                      (test rig only): skip the sidecar and the exhibit config,
                      take --fps/--mode as given
  --test-rig-force-recovery-after-secs N
                      (test rig only): force an in-place recovery N seconds
                      after the loadfile request (not N seconds of confirmed
                      playback -- decode startup takes time too), whether or not
                      anything has stalled. To catch problems in steady playback
                      rather than at startup, pick N with margin over real
                      decode startup latency. Requires --test-rig-no-sidecar
                      (refused otherwise) so it can never fire against a real,
                      sidecar-bound deployment asset.
  --test-rig-hang-after-secs N
                      (test rig only): N seconds after startup, hang the
                      supervisor thread forever -- the one hazard neither the
                      health check nor the heartbeat can see (a
                      supervisor-thread hang outside any mpv call), to prove
                      whether a systemd watchdog (WatchdogSec=) fires and
                      restarts this process. The process never recovers on its
                      own once this fires; only an external actor (systemd, or a
                      test harness's own kill) can end it. Requires
                      --test-rig-no-sidecar (refused otherwise) so it can never
                      fire against a real, sidecar-bound deployment asset.
  --opt K=V           pass an extra mpv option (repeatable)
  --no-defaults       omit the built-in Pi 4 zero-copy option set

exit codes: 2 = refused before playback (bad invocation/asset/sidecar/display; fix and redeploy)
            1 = playback/runtime failure (systemd restarts this process)"
    );
    std::process::exit(2)
}

/// Locate `/sys/class/drm/card<N>-<connector>/modes`. The card number is found
/// by search: vc4/v3d probe order makes it unstable across kernel versions,
/// which is why `deploy/dex-wait-hdmi` globs it too. Returns `None` when no such
/// entry exists -- no DRM at all (a CI container), or a mistyped connector
/// name.
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
    // Print the build identity before anything can fail: a log that opens with
    // an unidentified process cannot be read back weeks later.
    eprintln!(
        "dexd {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("DEX_GIT_HASH")
    );

    let args: Vec<String> = env::args().skip(1).collect();
    // An empty argv is the normal deployment invocation
    // (`ExecStart=/usr/bin/dexd`, everything else in
    // /opt/dex/exhibit.{yaml,json}), so there is no "no arguments prints usage"
    // guard here: a bare `dexd` proceeds to the config and refuses there if the
    // config cannot answer.
    // See docs/design/startup-checks.md#argument-shape.

    let mut path: Option<String> = None;
    let mut cli_fps: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut extra: Vec<(String, String)> = Vec::new();
    let mut defaults = true;
    let mut test_rig_no_sidecar = false;
    let mut test_rig_force_recovery_after_secs: Option<u64> = None;
    let mut test_rig_hang_after_secs: Option<u64> = None;
    let mut exhibit_config_path: Option<String> = None;
    // Test-only override so integration tests can supply a synthetic kernel
    // cmdline instead of depending on the host's /proc/cmdline, which varies by
    // machine (a CI container has no video= token at all). Not printed in
    // usage(): a deployment always reads the real /proc/cmdline.
    let mut proc_cmdline_path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fps" => {
                i += 1;
                // A missing value here -- the flag as the last token, from an
                // edited systemd unit or a line-continuation typo -- refuses
                // loudly. A dropped --fps would fall through to "no
                // cross-check" and a dropped --mode to the connector's
                // preferred mode: the wrong cadence or the wrong resolution,
                // forever, with no error.
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
            "--test-rig-no-sidecar" => test_rig_no_sidecar = true,
            "--test-rig-force-recovery-after-secs" => {
                i += 1;
                // Same discipline as --fps and --mode above: a missing value
                // leaves the probe disarmed, which is harmless, but a
                // malformed value must not read as "flag absent" either, so
                // both go to usage().
                let Some(v) = args.get(i) else { usage() };
                let Ok(n) = v.parse::<u64>() else { usage() };
                test_rig_force_recovery_after_secs = Some(n);
            }
            "--test-rig-hang-after-secs" => {
                i += 1;
                // Same discipline as --test-rig-force-recovery-after-secs above.
                let Some(v) = args.get(i) else { usage() };
                let Ok(n) = v.parse::<u64>() else { usage() };
                test_rig_hang_after_secs = Some(n);
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

    // No guard on a missing positional path: the asset may come from the
    // exhibit config, so which asset plays is a resolution
    // (exhibit::resolve_asset, below, once the config is read) and not a shape
    // of argv. A refusal here would reject the normal deployment invocation,
    // `ExecStart=/usr/bin/dexd` with no path at all.

    // `--test-rig-force-recovery-after-secs` requires `--test-rig-no-sidecar`.
    // The pairing ties the probe to the flag that already keeps test runs out
    // of every deployment -- deploy/dexd.service never passes it, because a
    // real asset is bound to its sidecar -- so the probe cannot end up armed
    // against a show by someone pasting a test-rig command line into the wrong
    // place. Checked before the asset is read, so the refusal rests on the
    // command line alone and not on whether a sidecar exists on disk.
    // See docs/design/startup-checks.md#test-rig-only-override.
    if test_rig_force_recovery_after_secs.is_some() && !test_rig_no_sidecar {
        eprintln!(
            "error: --test-rig-force-recovery-after-secs requires --test-rig-no-sidecar -- it \
             is a test-rig-only probe that forces an in-place recovery on a timer, whether or \
             not anything has stalled, and must never be armed against what could be a real, \
             sidecar-bound deployment asset. Add --test-rig-no-sidecar --fps <F> to run it on \
             a test rig, or drop --test-rig-force-recovery-after-secs to run normally."
        );
        return ExitCode::from(2);
    }

    // `--test-rig-hang-after-secs` requires `--test-rig-no-sidecar` for the same
    // reason: a probe that hangs the supervisor thread forever must never be
    // reachable against a real, sidecar-bound show, however it reached the
    // command line.
    if test_rig_hang_after_secs.is_some() && !test_rig_no_sidecar {
        eprintln!(
            "error: --test-rig-hang-after-secs requires --test-rig-no-sidecar -- it is a \
             test-rig-only probe that hangs the supervisor thread forever to prove whether a \
             systemd watchdog fires, and must never be armed against what could be a real, \
             sidecar-bound deployment asset. Add --test-rig-no-sidecar --fps <F> to run it on \
             a test rig, or drop --test-rig-hang-after-secs to run normally."
        );
        return ExitCode::from(2);
    }

    // The exhibit display config, read before the asset: these are the cheapest
    // startup checks and must not depend on the asset being present. Order:
    // config parse, cmdline check, sysfs mode pre-flight.
    // See docs/design/exhibit-config.md#exhibit-config.
    let found = if test_rig_no_sidecar {
        None
    } else {
        match load_exhibit_config(
            exhibit_config_path.as_deref(),
            &DEFAULT_EXHIBIT_CONFIG_PATHS,
        ) {
            Ok(found) => found,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        }
    };
    // When nothing was found there is no path to name, so the startup line
    // falls back to the default path -- which is also the file
    // resolve_display's refusal tells the operator to create.
    let (exhibit_config, exhibit_config_path) = match found {
        Some((cfg, path)) => (Some(cfg), path),
        None => (None, DEFAULT_EXHIBIT_CONFIG_PATH.to_string()),
    };

    let display = match resolve_display(
        exhibit_config.as_ref(),
        mode.as_deref(),
        test_rig_no_sidecar,
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // The cmdline check: keeps the exhibit config and the kernel `video=` token
    // in agreement. Skipped under --test-rig-no-sidecar, which by definition
    // runs outside the deployment config's authority, on hand-managed boot
    // state.
    if !test_rig_no_sidecar {
        let proc_cmdline_path = proc_cmdline_path
            .clone()
            .unwrap_or_else(|| "/proc/cmdline".to_string());
        match fs::read_to_string(&proc_cmdline_path) {
            Ok(cmdline) => {
                if let Err(e) =
                    check_cmdline_matches(&cmdline, &display.connector, &display.kms_force)
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

    // The sysfs mode pre-flight: catches a configured resolution the connected
    // display cannot show, before any asset, sidecar or leading-unit check
    // runs. Plain-text sysfs, no privilege, no libmpv --
    // `/sys/class/drm/card*-<connector>/modes`, one WxH per line. It runs
    // whenever a specific mode is requested, on a test rig too: it reads
    // nothing from the config or /proc, so the test-rig flag does not exempt
    // it, unlike the cmdline check above.
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
        "dexd: display {} ({}), connector {}, kms-force {}",
        display.display_mode,
        match display.source {
            DisplaySource::Config => format!("exhibit config {exhibit_config_path}"),
            DisplaySource::Bench => "test rig override, unbound".to_string(),
        },
        display.connector,
        if test_rig_no_sidecar {
            "not checked (test rig)".to_string()
        } else {
            display.kms_force.clone()
        },
    );

    // Which asset plays, resolved from the exhibit config and the command line
    // by the same decision-table shape as the display and the frame rate
    // (exhibit::resolve_asset). This is what lets several assets sit in
    // /opt/dex with the exhibit choosing one.
    // See docs/design/exhibit-config.md#exhibit-config.
    let asset = match resolve_asset(
        exhibit_config.as_ref(),
        config_dir(&exhibit_config_path),
        path.as_deref(),
        test_rig_no_sidecar,
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let path = asset.path;
    eprintln!(
        "dexd: asset {path} ({})",
        match asset.source {
            AssetSource::Config => format!("exhibit config {exhibit_config_path}"),
            // A show runs off the config, so a path on the command line means
            // a test rig or a hand-started one-off; the system log says so.
            AssetSource::Cli => "command line, not the exhibit config".to_string(),
        }
    );

    // Read the asset once. These are small (measured: 1.3 MB at 1080p, 14.8 MB
    // at 4K for a 3 s test video) and holding the bytes in memory keeps the
    // filesystem off the hot path: no re-open, no page-cache dependency, no
    // stall at the loop point.
    // See docs/design/measurements.md#memory-and-process-cost.
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

    // Bind the asset to its sidecar. A raw Annex-B stream carries no
    // timestamps, so a wrong --fps plays slow or fast forever with no error and
    // every metric nominal -- the one failure that cannot be detected at all.
    // The frame rate therefore travels with the asset, bound by a sha256, and
    // an unbound asset is refused; `--test-rig-no-sidecar --fps F` is the
    // two-flag test-rig override.
    // See docs/design/sidecar.md#asset-binding.
    let sidecar_path = format!("{path}.json");
    let sidecar: Option<Sidecar> = if test_rig_no_sidecar {
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
                     an asset without its sidecar is unbound (fps would be a \
                     guess); prepare the video again with dex-sidecar write to \
                     produce it, or use --test-rig-no-sidecar --fps <F> on a test rig"
                );
                return ExitCode::from(2);
            }
        }
    };

    let (fps, fps_source) = match resolve_fps(
        sidecar.as_ref().map(|s| s.fps.as_str()),
        cli_fps.as_deref(),
        test_rig_no_sidecar,
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

    // A display_mode of "auto" names no mode, so it skips the sysfs pre-flight
    // above. The sidecar is bound to the asset and names its resolution, so
    // warn when the connector cannot offer it. Two different things follow,
    // and both are bad in a way that is hard to read from the outside: on
    // hardware that builds no 4K mode unforced the artwork plays at whatever
    // the connector negotiates, for weeks, with every metric nominal; on a
    // connector whose modes are all smaller than the asset it does not play at
    // all, because this player's zero-copy path hands the decoded frame
    // straight to a plane and has no downscale step, so the clock never
    // advances and recovery escalates to a restart.
    //
    // A warning and not a refusal, because "auto" is also the only correct
    // answer for a panel whose one refresh is fractional -- there any integer
    // force fails mpv's mode match outright -- and a refusal here would reject
    // that configuration too. Whether "auto" should stay the default is open.
    // See docs/design/exhibit-config.md#exhibit-config and
    // docs/design/roadmap.md#open-questions.
    if display.display_mode == "auto" {
        if let Some((w, h)) = sidecar.as_ref().and_then(|s| s.width.zip(s.height)) {
            let want = format!("{w}x{h}");
            if let Some(modes_path) = find_sysfs_modes_path(&display.connector) {
                if let Ok(modes_text) = fs::read_to_string(&modes_path) {
                    if !sysfs_modes_contains(&modes_text, &want) {
                        let offered: Vec<&str> = modes_text.lines().map(str::trim).collect();
                        eprintln!(
                            "warning: display_mode is \"auto\" and the asset is {want} (per its \
                             sidecar), but connector {} offers only {offered:?} ({modes_path}). \
                             KMS drives some other mode, and this ends one of two ways: the \
                             artwork plays at the wrong resolution with every metric nominal, or \
                             it does not present at all and the player restarts on a clock that \
                             never advances -- the zero-copy path has no downscale step, so an \
                             asset larger than every mode the connector offers cannot be shown. \
                             Which fix applies is a property of the display. If it can build \
                             {want} once forced (some capture devices build no 4K mode unforced), \
                             set display_mode and kms_force in the exhibit config, run \
                             'sudo dex-exhibit-apply', and reboot. If it cannot build {want} at \
                             all, {want} is the wrong geometry for this display: prepare the \
                             asset at a resolution the connector offers",
                            display.connector
                        );
                    }
                }
            }
        }
    }

    // Validate the leading units of the stream. The loop is gapless only
    // because byte 0 begins with parameter sets and an IDR; an intact but wrong
    // asset (open GOP, no leading IDR, not Annex-B at all) would glitch at
    // every loop point, with nothing logged, about 29k times a day for a
    // 3-second video (derived). The checksum above catches truncation; this
    // catches shape. It runs on a test rig too.
    // See docs/design/startup-checks.md#order-of-checks.
    if let Err(e) = validate_leading_nals(leaked) {
        eprintln!("error: {path}: {e}");
        return ExitCode::from(2);
    }

    eprintln!(
        "dexd: {} bytes, fps {fps} ({}), looping endlessly",
        leaked.len(),
        match fps_source {
            FpsSource::Sidecar => "sidecar",
            FpsSource::BenchOverride => "test rig override, unbound",
        }
    );

    if let Some(n) = test_rig_force_recovery_after_secs {
        // The check above makes this unreachable without --test-rig-no-sidecar,
        // and a run that forces its own recovery belongs in the system log in
        // plain words, not inferred afterwards from a recovery line with no
        // explanation.
        eprintln!(
            "warning: (test rig only) (forced-recovery probe): \
             --test-rig-force-recovery-after-secs={n} is armed -- this run will force an \
             in-place recovery {n}s after the loadfile request was queued (not {n}s of \
             confirmed playback -- decode startup can itself take a few seconds, so a small N \
             can fire during startup rather than steady playback), whether or not anything has \
             stalled. Never pass this flag on a real deployment asset."
        );
    }

    if let Some(n) = test_rig_hang_after_secs {
        // Same discipline as the forced-recovery probe above.
        eprintln!(
            "warning: (test rig only) (hang probe): --test-rig-hang-after-secs={n} is armed -- \
             this run will hang the supervisor thread forever {n}s after startup, standing in \
             for a supervisor-thread hang outside any mpv call. \
             Never pass this flag on a real deployment asset."
        );
    }

    let ctx = unsafe { mpv_create() };
    if ctx.is_null() {
        eprintln!("error: mpv_create failed");
        return ExitCode::FAILURE;
    }

    let mut opts: Vec<(String, String)> = Vec::new();
    if defaults {
        // Measured on a Raspberry Pi 4 running trixie. The decoder emits
        // Broadcom SAND-tiled NV12 and the display scans SAND out natively, but
        // only straight onto a KMS plane; every other path detiles and loses
        // most of the frame rate. `drmprime-overlay` is the interop that puts
        // the frame on a plane; plain `drmprime` imports into GL.
        // See docs/design/measurements.md#measurement-record.
        for (k, v) in [
            ("vo", "gpu"),
            ("hwdec", "drm"),
            ("gpu-context", "drm"),
            ("gpu-api", "opengl"),
            ("gpu-hwdec-interop", "drmprime-overlay"),
            // Video on the primary plane and mpv's graphics and on-screen
            // display on the overlay, swapped from mpv's defaults, which keeps
            // the 4K video off the V3D render path. Re-verify after a kernel
            // upgrade or on another DRM driver.
            // See docs/design/architecture.md#architecture.
            ("drm-draw-plane", "overlay"),
            ("drm-drmprime-video-plane", "primary"),
            ("video-sync", "display-resample"),
            // Without this, a decoder that cannot use the hardware path falls
            // back to software with no message and plays 4K30 at about 14 fps,
            // measured on a Raspberry Pi 4. Making it fatal turns an invisible
            // performance collapse into an END_FILE error, which is handled and
            // restartable.
            // See docs/design/measurements.md#playback-path-throughput.
            ("hwdec-software-fallback", "no"),
            ("fullscreen", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
            // A raw elementary stream has no timestamps; mpv must generate them.
            ("correct-pts", "no"),
            // A second bound rather than the effective one: mpv runs its
            // aggressive cache only for streams flagged as network, and a
            // stream_cb stream is not, so read-ahead is governed by
            // demuxer-readahead-secs.
            ("demuxer-max-bytes", "64MiB"),
            // The effective prefetch depth: one second of decoded-ahead
            // insurance across the loop point.
            ("demuxer-readahead-secs", "1.0"),
        ] {
            opts.push((k.to_string(), v.to_string()));
        }
    }
    opts.push(("container-fps-override".into(), fps));
    // The resolved exhibit display binding. "auto" means: pass no drm-mode at
    // all, which is mpv's own documented default (drm-mode=preferred) -- see
    // exhibit::resolve_display for what "auto" means precisely. drm-connector is
    // passed unconditionally, because every ResolvedDisplay carries a connector
    // (defaulted when the config does not name one), so the choice is always
    // explicit.
    if display.display_mode != "auto" {
        opts.push(("drm-mode".into(), display.display_mode.clone()));
    }
    opts.push(("drm-connector".into(), display.connector.clone()));
    opts.extend(extra);

    for (k, v) in &opts {
        if let Err(e) = set_opt(ctx, k, v) {
            eprintln!("error: {e}");
            unsafe { mpv_terminate_destroy(ctx) };
            // A rejected option is deterministic: the same asset and flags
            // fail identically on every restart, so the exit-code contract
            // makes this a bad invocation (2) and not a runtime failure (1)
            // that a restart could resolve.
            return ExitCode::from(2);
        }
    }

    // Register `loop://` before initialize, so the protocol exists by the time
    // the play command is issued.
    //
    // `user_data` is a leaked Box and not a pointer to a local: mpv keeps this
    // pointer until mpv_terminate_destroy returns and may dereference it from
    // its own threads at any point, so a stack slot in main() would be
    // undefined behaviour the moment anything unwinds. 16 bytes, leaked once.
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
    // as events. It is what makes a failure at the venue diagnosable.
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

    // Subscribe to time-pos so the event loop can detect a stalled decode
    // without ever polling the core synchronously. This call is documented
    // non-blocking; the samples arrive as ordinary MPV_EVENT_PROPERTY_CHANGE
    // events through the same wait loop. A failure to register is not fatal,
    // because the health check is a safety net on top of a working player, but
    // it is logged.
    // See docs/design/failure-handling.md#health-check.
    let health_check_registered =
        observe(ctx, HEALTH_CHECK_USERDATA, "time-pos", MPV_FORMAT_DOUBLE);
    // Whether the health check is usable this run. The tick loop below runs
    // only when it is set (`health: Option<HealthMonitor>`): without that
    // condition, a failed registration would leave `last_position` permanently
    // `None`, every tick would read as a stall, and recoveries would be issued
    // against a healthy player. `observe()` has already logged the mpv error;
    // this is what that failure means for the feature.
    if !health_check_registered {
        eprintln!(
            "warning: the health check is disabled for this run (time-pos \
             subscription failed above); systemd still restarts this process on \
             a fatal event"
        );
    }

    // The heartbeat's two drop counters, subscribed the same way as time-pos
    // above. Registration order relative to `loadfile` does not affect
    // completeness: mpv forces an initial notification for every observer at
    // the moment it registers (client.h). That forced event arrives as
    // MPV_FORMAT_NONE with a null pointer while no video output chain exists
    // yet (see the property-change handler below); the first real value is a
    // second, later event. A failed registration is not fatal -- `observe()`
    // has already warned -- and means this run's heartbeat prints "off" for
    // that counter.
    let mut frame_drops = if observe(
        ctx,
        FRAME_DROPS_USERDATA,
        "frame-drop-count",
        MPV_FORMAT_INT64,
    ) {
        ObservedCounter::observed()
    } else {
        ObservedCounter::unobserved()
    };
    let mut vo_delayed = if observe(
        ctx,
        VO_DELAYED_USERDATA,
        "vo-delayed-frame-count",
        MPV_FORMAT_INT64,
    ) {
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

    // Run until mpv stops playing. The stream is endless by construction, so
    // there is no benign way for playback to end: an end-file event means
    // something failed, and it is fatal here.
    //
    // With vo=gpu the video output is created during file load and not during
    // mpv_initialize, so every display-side failure -- a projector not awake,
    // no EDID, DRM master held by a getty -- passes both the initialize and
    // loadfile return codes and lands here.
    // See docs/design/failure-handling.md#fatal-events.
    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    // Health-check state. `last_position` is updated only by the
    // MPV_EVENT_PROPERTY_CHANGE handler below, never read synchronously from
    // mpv, and fed to `health` on a fixed cadence. The `Instant` travels in the
    // same tuple as the position, so the two cannot desync.
    let mut last_position: Option<(f64, Instant)> = None;
    // Resolve the systemd watchdog handshake and open its socket, when armed,
    // before heartbeat #0, so that first line reports the real watchdog state.
    let mut watchdog_runtime = setup_watchdog(HEALTH_CHECK_SECS);
    // Heartbeat #0 proves the temperature reading and the line format on every
    // boot, and anchors the log. It does not prove the mpv property
    // subscriptions: frame drops, delayed frames and position all read "n/a"
    // here, because nothing has been decoded yet.
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
    // Armed only when the check above accepted
    // --test-rig-force-recovery-after-secs, which itself required
    // --test-rig-no-sidecar. `None` on every deployment run, at the cost of one
    // `Option` check per loop iteration.
    let mut force_recovery_trigger: Option<ForceRecoveryTrigger> =
        test_rig_force_recovery_after_secs.map(ForceRecoveryTrigger::new);
    // The test-rig hang probe, armed only when the check above accepted
    // --test-rig-hang-after-secs, which requires --test-rig-no-sidecar in the
    // same way. See its firing site below.
    let mut test_rig_hang_trigger: Option<ForceRecoveryTrigger> =
        test_rig_hang_after_secs.map(ForceRecoveryTrigger::new);
    // Counts recoveries issued whose matching END_FILE(reason=stop) has not
    // been observed yet. A count and not a flag; see
    // `is_expected_recovery_stop`. Nothing else in this program issues a
    // command that produces a stop-reason end-file, so a count above zero is
    // what separates an expected teardown from a real failure.
    let mut recovery_stops_pending: u32 = 0;

    let exit = ExitCode::SUCCESS;
    loop {
        // Wake at least every HEALTH_CHECK_SECS. In the healthy steady state
        // mpv delivers frequent time-pos property-change events that wake this
        // thread without help from the timeout; during a stall no such events
        // arrive, and that absence is the stall signal, so the timeout is what
        // keeps the health check running on schedule in the one case that
        // matters. The wake costs one event drain and two comparisons, on the
        // supervisor thread, off the decode path.
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

        // The health-check tick and the watchdog ping share this cadence, but
        // the ping is not nested inside `if let Some(h) = health` below: if the
        // time-pos subscription failed to register, the health check is off
        // while the player may still be healthy, and stopping pings then would
        // turn a merely degraded run into a certain watchdog kill loop. The
        // ping is emitted after the tick and act_on_health_action pair for this
        // iteration has completed. That ordering, plus the recovery budget that
        // never refills, bounds how many pings a stalled player can emit before
        // it exits or stops pinging, so the loop cannot both hang and keep
        // pinging.
        // See docs/design/failure-handling.md#systemd-watchdog.
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

        // The forced-recovery probe, checked every iteration -- two integer
        // comparisons, and the whole block skipped on a deployment run, where
        // `ForceRecoveryTrigger` is `None` -- so it does not additionally wait
        // out whatever remains of the current cadence window. It can still be
        // delayed by up to HEALTH_CHECK_SECS, because mpv_wait_event's timeout
        // bounds how often this loop body runs when nothing else wakes it.
        if let Some(trigger) = force_recovery_trigger.as_mut() {
            if trigger.should_fire(started.elapsed().as_secs()) {
                match health.as_mut() {
                    Some(h) => {
                        let action = h.force_recovery();
                        act_on_health_action(
                            ctx,
                            action,
                            "test rig: --test-rig-force-recovery-after-secs elapsed",
                            &mut last_position,
                            &mut recovery_stops_pending,
                        );
                    }
                    None => {
                        // The time-pos subscription failed at startup and was
                        // warned about there, so there is no HealthMonitor to
                        // spend a recovery attempt from and the probe is inert.
                        // Say so: an operator watching a run that never fires
                        // needs to know why.
                        eprintln!(
                            "warning: --test-rig-force-recovery-after-secs elapsed \
                             but cannot fire: \
                             the health check is disabled for this run (time-pos \
                             subscription failed above)"
                        );
                    }
                }
            }
        }

        // The test-rig hang probe parks this supervisor thread forever, standing
        // in for a hang in this program's own code outside any mpv call.
        // Checked every iteration, like the probe above, and placed before the
        // event-id dispatch below: on a run where mpv reaches END_FILE or
        // MPV_EVENT_QUEUE_OVERFLOW almost immediately (this file's own
        // `--opt vid=no --opt aid=no` test convention does that), the
        // dispatch's own `std::process::exit(1)` would win the race and the
        // probe would never fire. Firing parks the thread permanently -- a real
        // `loop`, not a long sleep -- so only an external actor (systemd's
        // watchdog when armed, otherwise the test harness's deadline kill) can
        // end this process from there.
        // See docs/design/development.md#building-and-testing-dexd.
        if let Some(trigger) = test_rig_hang_trigger.as_mut() {
            if trigger.should_fire(started.elapsed().as_secs()) {
                eprintln!(
                    "warning: (test rig only) (hang probe): \
                     --test-rig-hang-after-secs elapsed -- \
                     parking the supervisor thread forever, standing in for a supervisor-thread \
                     hang outside any mpv call. If a systemd watchdog \
                     is armed above, it should fire and this process should be restarted; \
                     if this process is still alive well past WatchdogSec, the \
                     watchdog did not fire."
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
            // Check both the reply_userdata tag and the format tag before
            // touching `data`: `data`'s type is decided by `format`. A mismatch
            // on either tag is not acted on. The two drop-counter observers
            // follow the same discipline for MPV_FORMAT_INT64, and also act on
            // MPV_FORMAT_NONE, see below.
            // See docs/design/failure-handling.md#health-check.
            if reply_userdata == HEALTH_CHECK_USERDATA
                && p.format == MPV_FORMAT_DOUBLE
                && !p.data.is_null()
            {
                // SAFETY: the format tag confirms `data` points to an f64,
                // per client.h's mpv_event_property contract for
                // MPV_FORMAT_DOUBLE. `.cast::<f64>()` rather than
                // `as *const f64`: this crate casts every raw pointer with
                // `.cast()`, so a pointee-type mismatch is a compile error
                // instead of an unreported reinterpretation.
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
                // A property that is momentarily unavailable -- no video output
                // chain before the first frame, and during a recovery's
                // teardown -- arrives as MPV_FORMAT_NONE with a null data
                // pointer (player/client.c:1810-1816). `total` survives that
                // untouched and `last_raw` is cleared, so the next sample,
                // however small, reads as a fresh first sample; see
                // `ObservedCounter::mark_unavailable`.
                // See docs/design/failure-handling.md#residual-gaps.
                match reply_userdata {
                    FRAME_DROPS_USERDATA => frame_drops.mark_unavailable(),
                    VO_DELAYED_USERDATA => vo_delayed.mark_unavailable(),
                    _ => {}
                }
            }
            continue;
        }
        if id == MPV_EVENT_COMMAND_REPLY {
            // Diagnostic only: nothing gates on this. The next health-check
            // tick judges the recovery by whether time-pos advances again, so
            // logging a rejection only makes that judgement readable in the
            // system log afterwards.
            if reply_userdata == RECOVERY_COMMAND_USERDATA {
                let error = unsafe { (*ev).error };
                if error < 0 {
                    eprintln!(
                        "dexd: health check: in-place recovery's loadfile command \
                         was rejected: {}",
                        err(ctx, "mpv_command_async(loadfile) reply", error)
                    );
                    // Rejected after being queued (mpv_command_async itself
                    // returned success) but before taking effect: no matching
                    // END_FILE(reason=stop) will arrive for this attempt, so
                    // its slot must not stay outstanding -- a count left too
                    // high could mask a later, unrelated stop as an expected
                    // teardown. Decrement by one and do not reset to zero:
                    // RECOVERY_COMMAND_USERDATA is shared by every recovery
                    // command, so another attempt may still be in flight and
                    // its own stop is still owed.
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
                // The in-place recovery's `loadfile ... replace` produced the
                // END_FILE(reason=stop) mpv emits for the file being replaced:
                // the expected teardown half of a recovery in progress, and not
                // a failure. Absorb exactly one and do not reset to zero; see
                // `is_expected_recovery_stop`.
                recovery_stops_pending -= 1;
                eprintln!(
                    "dexd: health check: in-place recovery's loadfile replaced the \
                     stream; absorbing the expected END_FILE(reason=stop) for the file \
                     it replaced ({recovery_stops_pending} more still outstanding), not \
                     treating it as a failure"
                );
                continue;
            }
            let why = unsafe { CStr::from_ptr(mpv_error_string(ef.error)) };
            eprintln!(
                "dexd: fatal: playback ended (reason={}, error={}) -- an endless \
                 stream must never end; exiting so systemd restarts this process",
                ef.reason,
                why.to_string_lossy()
            );
            // std::process::exit, not `break`: see the Escalate arm above for
            // why the shared mpv_terminate_destroy() teardown is the wrong tool
            // on a fatal exit path. The same applies to the queue-overflow arm
            // below.
            std::process::exit(1);
        }
        if id == MPV_EVENT_QUEUE_OVERFLOW {
            // mpv's internal event ring fills at 1000 pending events and drops
            // every event after that, END_FILE included, until the client
            // drains back to empty (client.c send_event). There is no
            // reservation for fatal events, so a dropped END_FILE would leave
            // this program in mpv's idle mode forever. What was lost cannot be
            // known, so this is treated like END_FILE: exit and let
            // systemd restart the player.
            eprintln!(
                "dexd: fatal: mpv event queue overflowed -- at least one event was \
                 dropped and may have been the one that mattered; exiting so systemd \
                 restarts this process"
            );
            std::process::exit(1);
        }
    }

    unsafe { mpv_terminate_destroy(ctx) };
    exit
}

// ---------------------------------------------------------------------------
// These tests live in the binary target, so they link libmpv even though they
// call no mpv function; `cargo check --all-targets` type-checks them without
// linking. `cargo test --lib` does not run this module -- it runs only the
// tests under the library target (src/lib.rs and its submodules).
// See docs/design/development.md#building-and-testing-dexd.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // `read_fn` returning 0 for a zero-length request reads to mpv as final end
    // of file. `chunk.rs` locks in the pure boundary (`next_chunk(_, _, 0) ==
    // None`); this test locks in the shell around it, and it is the only test
    // in the crate that fails if `else { return 0; }` comes back -- mpv itself
    // rarely issues a zero-length read, so the integration suite cannot see it
    // either.
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
        assert_eq!(
            pos_after_first, 3,
            "the callback must thread position through the same cookie, not reset per call"
        );

        let r = read_fn(cookie, buf.as_mut_ptr().cast::<c_char>(), 4);
        assert_eq!(
            r, 2,
            "short read: only 2 bytes remain before the loop point"
        );
        assert_eq!(&buf[..2], &[40, 50]);
        let pos_after_second = unsafe { &*(cookie as *mut LoopStream) }.pos;
        assert_eq!(
            pos_after_second, 0,
            "the position must land back at 0, not at len"
        );

        unsafe { drop(Box::from_raw(cookie as *mut LoopStream)) };
    }

    // `loadfile ... replace`, which in-place recovery issues, makes mpv emit
    // END_FILE(reason=stop) for the file being replaced. Without
    // `is_expected_recovery_stop` the event loop treats every end-file as
    // fatal, so a recovery's own first step kills the process and in-place
    // recovery is unreachable through the real event loop. These three cases
    // enforce the condition.

    #[test]
    fn a_pending_recovery_absorbs_its_own_stop_reason_end_file() {
        assert!(is_expected_recovery_stop(1, MPV_END_FILE_REASON_STOP));
    }

    #[test]
    fn a_stop_reason_end_file_with_no_recovery_pending_stays_fatal() {
        // Nothing else in this program issues a command that produces a
        // stop-reason end-file -- but if one ever did, it must not be absorbed
        // just because the reason code matches.
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

    // The health-check tick and the forced-recovery probe can both issue a
    // recovery before either one's END_FILE arrives (mpv_command_async only
    // queues against a core that may still be busy from the first attempt),
    // producing two END_FILE(reason=stop) events for one episode. A single flag
    // absorbs the first and treats the second -- an expected teardown for a
    // recovery that just worked -- as fatal, killing a healthy-again process.
    // See the `recovery_stops_pending` comment in the event loop above.

    #[test]
    fn two_overlapping_recoveries_both_absorb_their_own_stop() {
        let mut pending: u32 = 0;
        pending += 1; // the health-check tick issues attempt 1
        pending += 1; // the forced-recovery probe issues attempt 2, same iteration
        assert_eq!(pending, 2);

        assert!(is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP));
        pending -= 1; // first END_FILE(stop) absorbed
        assert_eq!(pending, 1, "one recovery's stop is still owed");

        assert!(
            is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP),
            "the second stop must not be treated as fatal because the first one \
             already cleared the count -- the case a plain bool cannot \
             represent"
        );
        pending -= 1;
        assert_eq!(pending, 0);

        // Both attempts absorbed: a further stop-reason end-file with nothing
        // outstanding is a real failure.
        assert!(!is_expected_recovery_stop(
            pending,
            MPV_END_FILE_REASON_STOP
        ));
    }

    #[test]
    fn a_rejected_command_reply_decrements_by_one_not_to_zero() {
        // Mirrors the MPV_EVENT_COMMAND_REPLY handler: a rejection after
        // queueing means that attempt's stop will never arrive, but
        // RECOVERY_COMMAND_USERDATA is shared by every recovery command, so a
        // second, still-legitimate attempt may be in flight and its stop must
        // remain expected.
        let mut pending: u32 = 2;
        pending = pending.saturating_sub(1); // one attempt's reply was rejected
        assert_eq!(pending, 1);
        assert!(is_expected_recovery_stop(pending, MPV_END_FILE_REASON_STOP));
    }
}
