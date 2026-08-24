//! Checks the hand-transcribed libmpv constants against the linked library.
//!
//! This target links libmpv, so it builds and runs only where libmpv-dev is
//! installed; elsewhere the library tests run with `cargo test --lib`. These
//! tests create no mpv instance and never touch the display, because
//! `mpv_event_name()` and `mpv_error_string()` are table lookups.
//!
//! See docs/design/architecture.md#language-and-bindings.

use std::ffi::{c_char, c_int, CStr};

use dexd::ffi_consts::{
    MPV_ERROR_UNSUPPORTED, MPV_EVENT_COMMAND_REPLY, MPV_EVENT_END_FILE, MPV_EVENT_LOG_MESSAGE,
    MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_QUEUE_OVERFLOW, MPV_EVENT_SHUTDOWN,
    MPV_EVENT_START_FILE,
};

#[link(name = "mpv")]
extern "C" {
    fn mpv_event_name(event: c_int) -> *const c_char;
    fn mpv_error_string(error: c_int) -> *const c_char;
}

/// Returns the library's name for an event id, or `None` for an id it does not
/// know, for which `mpv_event_name` returns a null pointer.
fn event_name(id: c_int) -> Option<String> {
    let p = unsafe { mpv_event_name(id) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// Returns the library's message for an error code. `mpv_error_string` always
/// returns a valid string, so the pointer needs no null check.
fn error_string(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn event_ids_match_the_live_library() {
    // mpv 0.40.0: 0 none, 1 shutdown, 2 log-message, 6 start-file, 7 end-file.
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

/// Checks the two event ids the health check dispatches on: property-change
/// carries the sampled playback position from `mpv_observe_property("time-pos",
/// ...)`, and command-reply says whether an asynchronous recovery command was
/// accepted.
#[test]
fn health_check_event_ids_match_the_live_library() {
    assert_eq!(
        event_name(MPV_EVENT_COMMAND_REPLY).as_deref(),
        Some("command-reply")
    );
    assert_eq!(
        event_name(MPV_EVENT_PROPERTY_CHANGE).as_deref(),
        Some("property-change")
    );
    // Both ids must also differ from every other id the event loop dispatches
    // on, so that no event's payload can be read as another event's struct.
    for other in [
        MPV_EVENT_NONE,
        MPV_EVENT_SHUTDOWN,
        MPV_EVENT_LOG_MESSAGE,
        MPV_EVENT_START_FILE,
        MPV_EVENT_END_FILE,
        MPV_EVENT_QUEUE_OVERFLOW,
    ] {
        assert_ne!(MPV_EVENT_COMMAND_REPLY, other);
        assert_ne!(MPV_EVENT_PROPERTY_CHANGE, other);
    }
    assert_ne!(MPV_EVENT_COMMAND_REPLY, MPV_EVENT_PROPERTY_CHANGE);
}

#[test]
fn queue_overflow_id_matches_the_live_library() {
    // mpv's event ring drops events, possibly an END_FILE, once 1000 are
    // pending; this id is the only signal that the drop happened, and main.rs
    // treats it as fatal. See docs/design/failure-handling.md#fatal-events.
    assert_eq!(
        event_name(MPV_EVENT_QUEUE_OVERFLOW).as_deref(),
        Some("event-queue-overflow")
    );
}

#[test]
fn log_message_and_start_file_ids_are_distinct() {
    // The event loop uses the id to decide which struct `mpv_event.data` points
    // at, so a collision between these two reads a start-file payload as a
    // log-message struct -- two unrelated integers as string pointers. Checked
    // in both directions, and against the library's own names.
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
    // mpv 0.40's string for -18 is "not supported". Compare the whole string,
    // not a substring: the error table also holds "unsupported format for
    // accessing option" (-6) and "...property" (-9), so a
    // `.contains("supported")` check would pass even if MPV_ERROR_UNSUPPORTED
    // were transcribed as -6 or -9.
    let s = error_string(MPV_ERROR_UNSUPPORTED);
    assert_eq!(s, "not supported", "mpv_error_string(-18) = {s:?}");
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-1),
        "-1 is EVENT_QUEUE_FULL, not MPV_ERROR_UNSUPPORTED"
    );
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-6),
        "-6 is OPTION_FORMAT, not MPV_ERROR_UNSUPPORTED"
    );
    assert_ne!(
        error_string(MPV_ERROR_UNSUPPORTED),
        error_string(-9),
        "-9 is PROPERTY_FORMAT, not MPV_ERROR_UNSUPPORTED"
    );
}
