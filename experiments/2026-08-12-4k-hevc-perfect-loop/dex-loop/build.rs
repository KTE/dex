//! Embed the git commit into the binary so the startup line identifies the
//! exact build. std only — zero dependencies — and a missing git or a
//! non-checkout build (e.g. the rsync'd Pi mirror) degrades to "nogit"
//! rather than failing the build. Ship builds should come from a checkout.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nogit".to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=DEX_GIT_HASH={hash}{}",
        if dirty { "+dirty" } else { "" }
    );
    // Re-run when HEAD moves (crate sits 3 levels below the repo root). The
    // paths may not exist in a non-checkout build; that is fine.
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/refs");
}
