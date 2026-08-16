//! Embed the git commit into the binary so the startup line identifies the
//! exact build. std only — zero dependencies.
//!
//! Three sources, in preference order:
//!  0. `DEX_BUILD_ID` — an environment variable. This exists because the file
//!     in (1) is NOT cache-safe: `rerun-if-changed` on a path that did not
//!     exist when the cached build ran is "simply never changed" (see the note
//!     below), so a CI job that restores a `target/` cache and THEN writes the
//!     stamp gets a stale binary reporting the old hash. Observed exactly that
//!     on 2026-08-16: the stamp was written, the package still said `nogit`.
//!     `rerun-if-env-changed` has no such hole — cargo compares the value.
//!  1. `.dex-build-id` — a one-line stamp file (gitignored) written by an
//!     external sync step. This matters because the Pi build is NOT a git
//!     checkout: it builds from an rsync mirror (`~/bench/dex-loop`), where
//!     `git rev-parse` fails and the shipped binary would otherwise always
//!     say "nogit" — every production binary permanently unidentifiable,
//!     exactly the failure F7 exists to prevent. Once the sync step writes
//!     the source commit hash here before each rsync, the Pi build picks it
//!     up automatically.
//!  2. `git rev-parse` against this checkout, when one exists (the Mac side,
//!     or any future build that IS a checkout).
//!
//! Falls back to "nogit" only when neither is available.

use std::process::Command;

fn main() {
    let hash = env_hash()
        .or_else(stamp_hash)
        .or_else(git_hash_with_dirty)
        .unwrap_or_else(|| "nogit".to_string());
    println!("cargo:rustc-env=DEX_GIT_HASH={hash}");

    // Cargo compares the VALUE of this variable, so it invalidates correctly
    // even from a warm cache — unlike a rerun-if-changed path that appears
    // where none existed.
    println!("cargo:rerun-if-env-changed=DEX_BUILD_ID");

    // Re-run on anything that could change the identity above. A missing
    // path is simply never "changed" -- fine, since not every build
    // environment has all three (the Pi mirror has no .git; a build with no
    // stamp file has no .dex-build-id).
    println!("cargo:rerun-if-changed=.dex-build-id");
    // Source edits must invalidate the cached hash: emitting ANY
    // rerun-if-changed replaces Cargo's default "rerun on any source
    // change", so without this line, editing src/*.rs and rebuilding WITHOUT
    // committing keeps reporting the previous (now stale) hash, with no
    // +dirty -- misattributing whatever the bench measures to code that
    // isn't in the binary.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    // Re-run when HEAD moves (crate sits 3 levels below the repo root). The
    // paths may not exist in a non-checkout build; that is fine.
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/refs");
}

fn env_hash() -> Option<String> {
    let s = std::env::var("DEX_BUILD_ID").ok()?;
    // Truncated to 12 to match the git path's --short=12: the startup line is
    // read by a human on site, and one shape is easier to compare against a
    // release note than "sometimes 12 chars, sometimes 40" depending on which
    // source happened to win.
    let s: String = s.trim().chars().take(12).collect();
    (!s.is_empty()).then_some(s)
}

fn stamp_hash() -> Option<String> {
    let s = std::fs::read_to_string(".dex-build-id").ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

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
