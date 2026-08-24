//! Build script: writes the commit into the binary as `DEX_GIT_HASH`, which
//! `dexd --version` and the first log line print. Standard library only, no
//! crates.
//!
//! Sources, in order: the `DEX_BUILD_ID` environment variable, the
//! `.dex-build-id` file, then `git rev-parse` in a checkout. With none of them
//! the build reports `nogit`, and the package on the device names no commit.
//!
//! See docs/design/packaging.md#version-and-build-identity.

use std::process::Command;

fn main() {
    let hash = env_hash()
        .or_else(stamp_hash)
        .or_else(git_hash_with_dirty)
        .unwrap_or_else(|| "nogit".to_string());
    println!("cargo:rustc-env=DEX_GIT_HASH={hash}");

    // Cargo compares the value of this variable, so a build from a warm cache
    // still picks up a new build id. CI sets this variable.
    println!("cargo:rerun-if-env-changed=DEX_BUILD_ID");

    // Rebuild when the build-id file changes. A path that does not exist counts
    // as never changed, so a file written after a cached `target/` was restored
    // does not invalidate it; use `DEX_BUILD_ID` above where that matters.
    println!("cargo:rerun-if-changed=.dex-build-id");

    // Emitting any rerun-if-changed replaces Cargo's default rebuild on source
    // change; these two lines put it back. Without them an edit that is not
    // committed keeps the previous hash. Keep them when adding another path.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Rebuild when the checkout moves to another commit; the crate sits two
    // levels below the repository root. Neither path exists in a build that is
    // not a checkout.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
}

/// Reads the build id from the `DEX_BUILD_ID` environment variable.
fn env_hash() -> Option<String> {
    let s = std::env::var("DEX_BUILD_ID").ok()?;
    // Trimmed to 12 characters to match the `--short=12` of the git path below,
    // so a technician comparing the startup line against a release note sees one
    // shape whichever source supplied it.
    let s: String = s.trim().chars().take(12).collect();
    (!s.is_empty()).then_some(s)
}

/// Reads the build id from `.dex-build-id`, a one-line file written by an
/// external sync step for builds that are not a git checkout.
fn stamp_hash() -> Option<String> {
    let s = std::fs::read_to_string(".dex-build-id").ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Reads the build id from git, appending `+dirty` when the checkout has
/// uncommitted changes.
fn git_hash_with_dirty() -> Option<String> {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    Some(format!("{hash}{}", if dirty { "+dirty" } else { "" }))
}
