//! libmpv event, error and format constants, transcribed from `mpv/client.h`
//! (mpv 0.40.0). `tests/ffi_constants.rs` checks the event and error values
//! against the linked library through `mpv_event_name()` and
//! `mpv_error_string()`; mpv exposes no name lookup for the format and
//! end-of-file tags, so those are transcription only and every use checks the
//! tag before it reads a payload. The ids do not run in sequence, so take a
//! new value from `client.h` instead of continuing a series.
//! See docs/design/architecture.md#language-and-bindings.

use std::ffi::c_int;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
/// Reply to a command sent with `mpv_command_async`. In-place recovery issues
/// its `loadfile` through the async command API so the supervisor thread keeps
/// detecting fatal events; this event only logs whether mpv accepted the
/// command, and no decision depends on it.
/// See docs/design/failure-handling.md#in-place-recovery.
pub const MPV_EVENT_COMMAND_REPLY: c_int = 5;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
/// Delivered when a property registered with `mpv_observe_property` changes
/// value. dexd learns `time-pos` and the drop counters from these events; the
/// runtime path contains no synchronous property read.
/// See docs/design/failure-handling.md#health-check.
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
/// "At least one event had to be dropped": mpv's 1000-slot event ring filled
/// and began discarding events, possibly including an `END_FILE`. The event
/// loop in main.rs treats it as fatal.
/// See docs/design/failure-handling.md#fatal-events.
pub const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;

/// The documented "not supported" return value for a stream callback. mpv 0.40
/// tests only the sign of the return value, so a plain `-1` also works; return
/// this constant, which stays correct if that test changes.
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;

/// `mpv_format` tag for a floating-point property value, the tag `time-pos`
/// arrives with. main.rs checks this tag on every `MPV_EVENT_PROPERTY_CHANGE`
/// payload before reading the payload as an `f64`.
/// See docs/design/failure-handling.md#health-check.
pub const MPV_FORMAT_DOUBLE: c_int = 5;

/// `mpv_format` tag for a 64-bit integer property value, used to observe the
/// two drop counters. Their native type in mpv is `int` and the client API
/// converts on the way out, so a 64-bit request is legal for them and the
/// payload is an `i64`.
/// See docs/design/failure-handling.md#heartbeat.
pub const MPV_FORMAT_INT64: c_int = 4;

/// `mpv_format` tag mpv substitutes for a property's real format when the
/// property is unavailable or its getter errored; the payload's `data` field
/// is invalid in that case (`client.h`). The drop counters arrive with this
/// tag while no video output chain exists, at startup and during an in-place
/// recovery, and main.rs clears their diffing baseline on it instead of
/// ignoring it.
/// See docs/design/failure-handling.md#heartbeat.
pub const MPV_FORMAT_NONE: c_int = 0;

/// `mpv_end_file_reason` tag for "playback was stopped by an external action"
/// (`client.h`). `loadfile <url> replace` delivers this reason for the file it
/// replaces, so main.rs absorbs it for a recovery it issued itself and treats
/// every other end-file reason, and every stop with no recovery pending, as
/// fatal.
/// See docs/design/failure-handling.md#expected-end-of-file.
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
