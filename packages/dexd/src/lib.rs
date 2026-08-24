//! dexd's pure core: everything the player decides without libmpv.
//!
//! The crate forbids unsafe code, so the FFI declarations and the mpv event
//! loop stay in `src/main.rs`.
//!
//! See docs/design/architecture.md#crate-layout.

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
