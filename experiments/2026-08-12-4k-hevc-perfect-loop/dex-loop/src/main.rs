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

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;

#[link(name = "mpv")]
extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(ctx: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut MpvHandle);
    fn mpv_set_option_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *const *const c_char) -> c_int;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_error_string(error: c_int) -> *const c_char;
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

/// Copy the next bytes out of the loop, wrapping at the end.
///
/// **Never returns 0.** Zero means EOF to mpv, which is precisely the event that
/// causes the seam this program exists to avoid. At the end of the payload the
/// offset wraps to 0 and the next read continues from the start, so the decoder
/// receives the loop's leading IDR as an ordinary mid-stream keyframe.
///
/// A short read is legal, so the wrap does not need to be stitched across a
/// single call: returning the tail now and the head next time is correct and
/// keeps this function branch-light.
extern "C" fn read_fn(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    // SAFETY: `cookie` is the Box<LoopStream> leaked in `open_fn`, and mpv
    // guarantees it is passed back unmodified for the life of the stream.
    let s = unsafe { &mut *(cookie as *mut LoopStream) };

    if s.pos >= s.data.len() {
        s.pos = 0;
    }
    let want = nbytes as usize;
    let avail = s.data.len() - s.pos;
    let n = want.min(avail);
    if n == 0 {
        // Only reachable if the payload is empty, which is rejected at startup.
        return 0;
    }

    // SAFETY: mpv guarantees `buf` is writable for `nbytes`; `n <= nbytes` and
    // `n` bytes are readable from `data[pos..]`, and the ranges cannot overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(s.pos), buf as *mut u8, n);
    }
    s.pos += n;
    n as i64
}

/// Report the stream as unseekable, exactly like a pipe.
///
/// Deliberate: an mpv that believes it can seek will try to, and seeking is the
/// operation that produces the seam. Refusing here keeps the only available
/// behaviour "keep reading forwards".
extern "C" fn seek_fn(_cookie: *mut c_void, _offset: i64) -> i64 {
    -1
}

/// Report the size as unknown, again like a pipe.
///
/// Returning the payload length would let mpv compute a duration and a progress
/// position for a stream that has neither, and would invite it to treat the end
/// of the buffer as the end of the media.
extern "C" fn size_fn(_cookie: *mut c_void) -> i64 {
    -1
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
        "usage: dex-loop <stream.265> --fps <F> [--mode WxH@R] [--no-defaults] [--opt K=V ...]

  <stream.265>   raw Annex-B HEVC elementary stream, looped endlessly
  --fps F        frame rate; a raw stream carries no timestamps, so this is required
  --mode WxH@R   force a DRM mode, e.g. 3840x2160@30 (default: connector preferred)
  --opt K=V      pass an extra mpv option (repeatable)
  --no-defaults  omit the built-in Pi 4 zero-copy option set"
    );
    std::process::exit(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }

    let mut path: Option<String> = None;
    let mut fps: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut extra: Vec<(String, String)> = Vec::new();
    let mut defaults = true;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fps" => {
                i += 1;
                fps = args.get(i).cloned();
            }
            "--mode" => {
                i += 1;
                mode = args.get(i).cloned();
            }
            "--no-defaults" => defaults = false,
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

    let (Some(path), Some(fps)) = (path, fps) else {
        usage()
    };

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
    eprintln!("dex-loop: {} bytes, looping endlessly", leaked.len());

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
            // Video on the primary plane, mpv's GL/OSD surface on the overlay:
            // keeps the 4K video off the V3D render path entirely.
            ("drm-draw-plane", "overlay"),
            ("drm-drmprime-video-plane", "primary"),
            ("video-sync", "display-resample"),
            ("fullscreen", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
            // A raw elementary stream has no timestamps; mpv must generate them.
            ("correct-pts", "no"),
            // The stream never ends, so an unbounded cache would grow forever.
            ("demuxer-max-bytes", "64MiB"),
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
    let proto = CString::new("loop").unwrap();
    let mut data_ptr: &'static [u8] = leaked;
    let rc = unsafe {
        mpv_stream_cb_add_ro(
            ctx,
            proto.as_ptr(),
            &mut data_ptr as *mut &'static [u8] as *mut c_void,
            Some(open_fn),
        )
    };
    if rc < 0 {
        eprintln!("error: {}", err(ctx, "mpv_stream_cb_add_ro", rc));
        unsafe { mpv_terminate_destroy(ctx) };
        return ExitCode::FAILURE;
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

    // Run until mpv shuts down. There is no natural end -- the stream is
    // infinite -- so this only returns on an explicit quit or a fatal error.
    loop {
        let ev = unsafe { mpv_wait_event(ctx, -1.0) };
        if ev.is_null() {
            continue;
        }
        let id = unsafe { (*ev).event_id };
        if id == MPV_EVENT_SHUTDOWN {
            break;
        }
        if id == MPV_EVENT_NONE {
            continue;
        }
    }

    unsafe { mpv_terminate_destroy(ctx) };
    ExitCode::SUCCESS
}
