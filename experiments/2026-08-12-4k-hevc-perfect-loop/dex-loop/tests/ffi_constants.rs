//! T2 — verify the hand-transcribed FFI constants against the LIVE libmpv.
//!
//! This target links libmpv, so it builds and runs ONLY where libmpv-dev is
//! installed (the Pi: `cargo test`). On the Mac use `cargo test --lib`. It
//! creates no mpv instance and never touches the display: mpv_event_name()
//! and mpv_error_string() are static table lookups.
//!
//! Why this exists: MPV_EVENT_LOG_MESSAGE was once transcribed as 6 (it is 2;
//! 6 is START_FILE). The handler cast a start-file payload to a log-message
//! struct and segfaulted on the first frame. mpv exposes the id->name mapping
//! at runtime, so assert the transcription instead of trusting it — this also
//! catches a future mpv renumbering, which no header copy can.

use std::ffi::{c_char, c_int, CStr};

use dex_loop::ffi_consts::{
    MPV_ERROR_UNSUPPORTED, MPV_EVENT_END_FILE, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_QUEUE_OVERFLOW, MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
};

#[link(name = "mpv")]
extern "C" {
    fn mpv_event_name(event: c_int) -> *const c_char;
    fn mpv_error_string(error: c_int) -> *const c_char;
}

/// mpv_event_name returns NULL for ids it does not know.
fn event_name(id: c_int) -> Option<String> {
    let p = unsafe { mpv_event_name(id) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn error_string(code: c_int) -> String {
    // mpv_error_string is documented to always return a valid string.
    unsafe { CStr::from_ptr(mpv_error_string(code)) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn event_ids_match_the_live_library() {
    // Verified against mpv v0.40.0 on the bench Pi (2026-08-15, via ctypes):
    //   0 none, 1 shutdown, 2 log-message, 6 start-file, 7 end-file
    assert_eq!(event_name(MPV_EVENT_NONE).as_deref(), Some("none"));
    assert_eq!(event_name(MPV_EVENT_SHUTDOWN).as_deref(), Some("shutdown"));
    assert_eq!(
        event_name(MPV_EVENT_LOG_MESSAGE).as_deref(),
        Some("log-message")
    );
    assert_eq!(
        event_name(MPV_EVENT_START_FILE).as_deref(),
        Some("start-file")
    );
    assert_eq!(event_name(MPV_EVENT_END_FILE).as_deref(), Some("end-file"));
}

#[test]
fn queue_overflow_id_matches_the_live_library() {
    // mpv's internal event ring silently drops events (including a
    // potential END_FILE) once it chokes at 1000 pending; this id is the
    // only signal that happened. Pin it against the live library exactly
    // like the others above -- a mistranscription here would defeat the
    // fatal handling in main.rs just as silently as the LOG_MESSAGE bug did.
    assert_eq!(
        event_name(MPV_EVENT_QUEUE_OVERFLOW).as_deref(),
        Some("event-queue-overflow")
    );
}

#[test]
fn the_exact_bug_that_shipped_cannot_recur() {
    // The historical defect: LOG_MESSAGE transcribed as 6, which is
    // START_FILE. Pin the two apart, in both directions.
    assert_ne!(MPV_EVENT_LOG_MESSAGE, MPV_EVENT_START_FILE);
    assert_ne!(
        event_name(MPV_EVENT_LOG_MESSAGE),
        event_name(MPV_EVENT_START_FILE)
    );
    assert_ne!(
        event_name(6).as_deref(),
        Some("log-message"),
        "6 is start-file, not log-message"
    );
}

#[test]
fn error_unsupported_matches_the_live_library() {
    // The live mpv 0.40 string for -18 is "not supported" — NOT "unsupported";
    // PLAN.md's original assertion would have failed here. Exact match, not
    // a `.contains("supported")` hedge: err_table also has "unsupported
    // format for accessing option" (-6) and "...property" (-9), both of
    // which pass a loose "supported" substring check just as well as -18
    // does -- so a mistranscription of MPV_ERROR_UNSUPPORTED as -6 or -9
    // would sail through the exact test whose entire job is to catch that.
    // The whole philosophy here is "assert against the live library and let
    // drift fail loudly"; a wording-family hedge defeats it.
    let s = error_string(MPV_ERROR_UNSUPPORTED);
    assert_eq!(s, "not supported", "mpv_error_string(-18) = {s:?}");
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-1),
        "-1 is EVENT_QUEUE_FULL, the sign-compatible-but-wrong value this code once used"
    );
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-6),
        "-6 is OPTION_FORMAT, not UNSUPPORTED"
    );
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-9),
        "-9 is PROPERTY_FORMAT, not UNSUPPORTED"
    );
}
