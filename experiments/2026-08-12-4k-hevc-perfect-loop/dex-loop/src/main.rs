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
use dex_loop::ffi_consts::{
    MPV_ERROR_UNSUPPORTED, MPV_EVENT_END_FILE, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_QUEUE_OVERFLOW, MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
};
use dex_loop::heartbeat::format_heartbeat;
use dex_loop::nal::validate_leading_nals;
use dex_loop::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar};
use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs;
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
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_request_log_messages(ctx: *mut MpvHandle, min_level: *const c_char) -> c_int;
    fn mpv_stream_cb_add_ro(
        ctx: *mut MpvHandle,
        protocol: *const c_char,
        user_data: *mut c_void,
        open_fn: Option<extern "C" fn(*mut c_void, *mut c_char, *mut MpvStreamCbInfo) -> c_int>,
    ) -> c_int;
    fn mpv_get_property_string(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_free(data: *mut c_void);
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
    // The `&mut` additionally requires exclusivity: stream_cb.h serializes
    // every callback for a given stream (read/seek/size/close never run
    // concurrently with each other for the same cookie), so no other
    // callback can be touching this LoopStream while this borrow is live.
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
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(c.start), buf as *mut u8, c.n);
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

/// Read an mpv property as a string, or None if unavailable. Used only by
/// the low-frequency heartbeat — never on the decode path.
fn get_prop(ctx: *mut MpvHandle, name: &str) -> Option<String> {
    let n = CString::new(name).ok()?;
    // SAFETY: ctx is a valid initialized handle; mpv returns NULL or a
    // NUL-terminated string that must be released with mpv_free.
    let p = unsafe { mpv_get_property_string(ctx, n.as_ptr()) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    // SAFETY: p came from mpv_get_property_string and is released exactly once.
    unsafe { mpv_free(p as *mut c_void) };
    Some(s)
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

/// One heartbeat line to stderr. Runs on the event thread, off the decode
/// path; two property reads and one sysfs read per 10 minutes is noise.
fn emit_heartbeat(ctx: *mut MpvHandle, started: Instant) {
    let drops = get_prop(ctx, "frame-drop-count");
    let delayed = get_prop(ctx, "vo-delayed-frame-count");
    eprintln!(
        "{}",
        format_heartbeat(
            WRAP_COUNT.load(Ordering::Relaxed),
            started.elapsed().as_secs(),
            read_temp_millicelsius(),
            drops.as_deref(),
            delayed.as_deref(),
        )
    );
}

fn usage() -> ! {
    eprintln!(
        "usage: dex-loop <stream.265> [--fps <F>] [--mode WxH@R] [--bench-no-sidecar] [--no-defaults] [--opt K=V ...]

  <stream.265>        raw Annex-B HEVC elementary stream, looped endlessly
  <stream.265>.json   ingest sidecar, REQUIRED: {{\"fps\":\"30\",\"sha256\":\"<64 hex>\"}}
                      fps comes from it; the sha256 must match the asset bytes
  --fps F             optional cross-check; must equal the sidecar fps exactly
  --mode WxH@R        force a DRM mode, e.g. 3840x2160@30 (default: connector preferred)
  --bench-no-sidecar  BENCH ONLY: skip the sidecar, take --fps as given
  --opt K=V           pass an extra mpv option (repeatable)
  --no-defaults       omit the built-in Pi 4 zero-copy option set

exit codes: 2 = refused before playback (bad invocation/asset/sidecar; fix and redeploy)
            1 = playback/runtime failure (the supervisor restarts)"
    );
    std::process::exit(2)
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
            "--no-defaults" => defaults = false,
            "--bench-no-sidecar" => bench_no_sidecar = true,
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
    if let Some(m) = mode {
        opts.push(("drm-mode".into(), m));
    }
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
    // Heartbeat #0: proves the whole mechanism (property reads, temperature,
    // formatting) on every boot, and anchors the journal.
    emit_heartbeat(ctx, started);

    let mut exit = ExitCode::SUCCESS;
    loop {
        // Wake at least every 30 s: in the healthy steady state mpv delivers
        // NO events, which is precisely when the heartbeat must still fire.
        // A 30 s wake on the event thread costs nothing on the decode path.
        let ev = unsafe { mpv_wait_event(ctx, 30.0) };
        let id = unsafe { (*ev).event_id };

        if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_SECS {
            emit_heartbeat(ctx, started);
            last_heartbeat = Instant::now();
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
            let why = unsafe { CStr::from_ptr(mpv_error_string(ef.error)) };
            eprintln!(
                "dex-loop: FATAL: playback ended (reason={}, error={}) -- an endless \
                 stream must never end; exiting so the supervisor restarts",
                ef.reason,
                why.to_string_lossy()
            );
            exit = ExitCode::FAILURE;
            break;
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
            exit = ExitCode::FAILURE;
            break;
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
        let r = read_fn(cookie, buf.as_mut_ptr() as *mut c_char, 0);
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

        let r = read_fn(cookie, buf.as_mut_ptr() as *mut c_char, 3);
        assert_eq!(r, 3);
        assert_eq!(&buf[..3], &[10, 20, 30]);
        // SAFETY: single-threaded test; no other call is touching `cookie`.
        let pos_after_first = unsafe { &*(cookie as *mut LoopStream) }.pos;
        assert_eq!(pos_after_first, 3, "the callback must thread position through the same cookie, not reset per call");

        let r = read_fn(cookie, buf.as_mut_ptr() as *mut c_char, 4);
        assert_eq!(r, 2, "short read: only 2 bytes remain before the wrap");
        assert_eq!(&buf[..2], &[40, 50]);
        let pos_after_second = unsafe { &*(cookie as *mut LoopStream) }.pos;
        assert_eq!(pos_after_second, 0, "eager wrap: position must land back at 0, not at len");

        unsafe { drop(Box::from_raw(cookie as *mut LoopStream)) };
    }
}
