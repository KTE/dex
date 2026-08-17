//! dexd's pure core: every piece of logic that can exist without libmpv.
//!
//! The binary (src/main.rs) is deliberately a thin unsafe shell over this
//! library: FFI structs, callbacks, and the event loop. Everything decidable —
//! wrap arithmetic, hashing, sidecar binding, NAL validation, log formatting —
//! lives here, testable on any machine with `cargo test --lib`, no libmpv and
//! no display required. That split is T0 of the hardening plan: every serious
//! bug so far lived where testing could not reach.

#![forbid(unsafe_code)]

pub mod chunk;
pub mod exhibit;
pub mod ffi_consts;
pub mod health;
pub mod heartbeat;
pub mod nal;
pub mod sha256;
pub mod sidecar;
pub mod watchdog;
