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
    MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
};
use dex_loop::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar};
use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs;
use std::process::ExitCode;

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
    fn mpv_set_option_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *const *const c_char) -> c_int;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_request_log_messages(ctx: *mut MpvHandle, min_level: *const c_char) -> c_int;
    fn mpv_stream_cb_add_ro(
        ctx: *mut MpvHandle,
        protocol: *const c_char,
        user_data: *mut c_void,
        open_fn: Option<
            extern "C" fn(*mut c_void, *mut c_char, *mut MpvStreamCbInfo) -> c_int,
        >,
    ) -> c_int;
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

/// The stream read callback: a thin unsafe shell over
/// [`dex_loop::chunk::next_chunk`], which owns (and tests) every rule that
/// matters — never return 0 (to mpv, 0 is final EOF, the one event this
/// program exists to prevent), wrap eagerly, report a zero-length request as
/// an error rather than 0, saturate the u64 request size. This function only
/// performs the memcpy the pure core cannot.
extern "C" fn read_fn(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    // SAFETY: `cookie` is the Box<LoopStream> leaked in `open_fn`, and mpv
    // guarantees it is passed back unmodified for the life of the stream.
    let s = unsafe { &mut *(cookie as *mut LoopStream) };

    let Some(c) = next_chunk(s.data.len(), s.pos, clamp_want(nbytes)) else {
        // Zero-length request (or an impossible empty payload). Report an
        // error, never 0.
        return i64::from(MPV_ERROR_UNSUPPORTED);
    };

    // SAFETY: mpv guarantees `buf` is writable for `nbytes` bytes; next_chunk
    // guarantees c.n >= 1, c.n <= nbytes (the request is clamped, never
    // grown) and c.start + c.n <= data.len(), and the ranges cannot overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(c.start), buf as *mut u8, c.n);
    }
    s.pos = c.next_pos;
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
                cli_fps = args.get(i).cloned();
            }
            "--mode" => {
                i += 1;
                mode = args.get(i).cloned();
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
            return ExitCode::FAILURE;
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
    let mut exit = ExitCode::SUCCESS;
    loop {
        let ev = unsafe { mpv_wait_event(ctx, -1.0) };
        let id = unsafe { (*ev).event_id };

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
    }

    unsafe { mpv_terminate_destroy(ctx) };
    exit
}
