//! libmpv ABI constants, transcribed from mpv/client.h (mpv v0.40.0) and —
//! more importantly — verified against the LIVE library at test time by
//! tests/ffi_constants.rs via mpv_event_name()/mpv_error_string().
//!
//! Why the paranoia: an earlier revision transcribed MPV_EVENT_LOG_MESSAGE as
//! 6. It is 2; 6 is MPV_EVENT_START_FILE. The event handler then cast a
//! start-file payload to a log-message struct and dereferenced garbage
//! pointers — a segfault on the first frame. A transcription can silently
//! rot; the live library cannot. These are NOT sequential-by-category.

use std::ffi::c_int;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;

/// The documented "not supported" sentinel for stream callbacks. `-1` is
/// MPV_ERROR_EVENT_QUEUE_FULL, which happens to work only because mpv 0.40
/// tests the sign rather than the value.
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;
