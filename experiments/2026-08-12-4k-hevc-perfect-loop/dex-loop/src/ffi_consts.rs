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
/// Delivered in reply to `mpv_command_async`. F1's tier-0 in-place recovery
/// issues its `loadfile` through the async command API specifically so the
/// event thread can never block on it (see src/health.rs's module doc);
/// this event is used only to log whether the queued command was accepted,
/// nothing gates on it.
pub const MPV_EVENT_COMMAND_REPLY: c_int = 5;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
/// Delivered when a property registered via `mpv_observe_property` changes
/// value (or, for some properties, at mpv's own internal polling cadence).
/// F1's health check subscribes to `time-pos` through this rather than
/// polling `mpv_get_property_string` synchronously -- see src/health.rs's
/// module doc for why that distinction matters.
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
/// "At least one event had to be dropped." Delivered once the internal
/// 1000-slot event ring chokes and starts silently discarding events --
/// including, potentially, an END_FILE. Treated as fatal: see the event
/// loop in main.rs.
pub const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;

/// The documented "not supported" sentinel for stream callbacks. `-1` is
/// MPV_ERROR_EVENT_QUEUE_FULL, which happens to work only because mpv 0.40
/// tests the sign rather than the value.
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;

/// `mpv_format` tag for a plain floating-point property value (client.h's
/// `mpv_format` enum: NONE=0, STRING=1, OSD_STRING=2, FLAG=3, INT64=4,
/// DOUBLE=5, NODE=6, ...). Unlike the event ids above, mpv exposes no
/// `mpv_format_name()` to verify this against the live library the same
/// way, so main.rs checks this tag on every `MPV_EVENT_PROPERTY_CHANGE`
/// payload before ever reading it as an `f64`, rather than trusting the
/// transcription blindly -- the same class of bug that made
/// MPV_EVENT_LOG_MESSAGE's mistranscription a segfault instead of a caught
/// error.
pub const MPV_FORMAT_DOUBLE: c_int = 5;

/// `mpv_format` tag for a 64-bit integer property value (client.h's
/// `mpv_format` enum: NONE=0, STRING=1, OSD_STRING=2, FLAG=3, INT64=4,
/// DOUBLE=5, ...). F9's two observed drop counters use it. Their NATIVE
/// type is `int` (`m_property_int_ro`, player/command.c:763-781), but the
/// client API converts on the way out -- getproperty_fn routes INT64
/// through M_PROPERTY_GET_NODE -> conv_node_to_format (player/client.c
/// :1417-1442), and m_property_do synthesizes GET_NODE from GET for the
/// int type (options/m_property.c:171-188) -- so INT64 is a legal request
/// for them and the payload is an i64.
///
/// Like MPV_FORMAT_DOUBLE this cannot be checked against the live library
/// (mpv exposes no mpv_format_name()), so main.rs checks the tag on every
/// payload before dereferencing it. A wrong value here fails in the SAFE
/// direction, exactly like MPV_END_FILE_REASON_STOP: the tag never
/// matches, the counters read "n/a" forever, and nothing is misread.
pub const MPV_FORMAT_INT64: c_int = 4;

/// `mpv_format` tag mpv substitutes for a property's real format whenever
/// the property is unavailable or a getter errored -- client.h: "Warning: if
/// a property is unavailable or retrieving it caused an error,
/// MPV_FORMAT_NONE will be set in mpv_event_property, even if the format
/// parameter was set to a different value. In this case, the
/// mpv_event_property.data field is invalid." F9's two observed drop
/// counters see this instead of MPV_FORMAT_INT64 whenever no vo_chain
/// exists (`M_PROPERTY_UNAVAILABLE`, player/command.c:763-781) -- at startup,
/// and during a tier-0 recovery's teardown/rebuild. main.rs's
/// property-change handler acts on it (see `ObservedCounter::mark_unavailable`)
/// rather than silently dropping it, so a reset that mpv's event coalescing
/// hides cannot silently under-count the heartbeat's totals.
pub const MPV_FORMAT_NONE: c_int = 0;

/// `mpv_end_file_reason` tag for MPV_EVENT_END_FILE's `reason` field:
/// "Playback was stopped by an external action" (client.h). Bench-confirmed
/// live on the Pi's mpv 0.40.0 (three independent adversarial reviews,
/// 2026-08-15, IPC/ctypes probes) that `loadfile <url> replace` delivers
/// exactly this reason for the file being replaced. F1's in-place recovery
/// issues that exact command, so main.rs must recognize and absorb this one
/// reason for precisely the one command it issued itself -- every other
/// reason, and every STOP with no recovery in flight, stays fatal. See
/// PLAN.md's F1 addendum for why treating ANY end-file as fatal made
/// tier-0 recovery unreachable before this constant existed.
///
/// Unlike the event ids above, mpv exposes no runtime name lookup for
/// END_FILE reasons, so this is transcription-only -- but a wrong value
/// here fails in the SAFE direction: main.rs's absorption check simply
/// never matches, so an unmatched STOP falls through to the pre-existing
/// fatal path exactly as it did before this feature existed.
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
