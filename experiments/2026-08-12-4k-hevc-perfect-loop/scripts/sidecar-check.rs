//! Round-trip checker for dex-loop's F3 sidecar contract.
//!
//! NOT part of the dex-loop crate and not built by its Cargo.toml. A
//! standalone single-file program that pulls in the crate's OWN
//! sidecar.rs/sha256.rs via #[path], so validation runs the real parser
//! (`Sidecar::from_json`, `verify_payload`) byte-for-byte -- not a bash or
//! jq reimplementation of the grammar that could silently drift from it.
//! This mirrors the crate's own pure-logic split (dex-loop/src/lib.rs):
//! sidecar.rs and sha256.rs need no libmpv, so this builds and runs on the
//! Mac (no libmpv, no DRM) exactly as it does on the Pi.
//!
//! Build: rustc --edition 2021 -o sidecar-check sidecar-check.rs
//! Usage: sidecar-check <sidecar.json> <stream-file>
//!   exit 0, "OK ..." on stdout   -- sidecar parses AND its sha256 matches
//!                                   the stream's exact on-disk bytes
//!   exit 1, error on stderr     -- whatever the real parser/verifier said
//!   exit 2                      -- usage error

#[path = "../dex-loop/src/sha256.rs"]
mod sha256;
#[path = "../dex-loop/src/sidecar.rs"]
mod sidecar;

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [sidecar_path, stream_path] = args.as_slice() else {
        eprintln!("usage: sidecar-check <sidecar.json> <stream-file>");
        return ExitCode::from(2);
    };

    let text = match fs::read_to_string(sidecar_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {sidecar_path}: {e}");
            return ExitCode::from(1);
        }
    };

    let parsed = match sidecar::Sidecar::from_json(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {sidecar_path}: {e}");
            return ExitCode::from(1);
        }
    };

    // The exact bytes the player reads: `fs::read(&path)` in main.rs, no
    // filtering. Same call here, on purpose.
    let payload = match fs::read(stream_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot read {stream_path}: {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = sidecar::verify_payload(&payload, &parsed) {
        eprintln!("error: {e}");
        return ExitCode::from(1);
    }

    println!(
        "OK fps={} sha256={} width={} height={}",
        parsed.fps,
        parsed.sha256,
        parsed
            .width
            .map(|w| w.to_string())
            .unwrap_or_else(|| "-".to_string()),
        parsed
            .height
            .map(|h| h.to_string())
            .unwrap_or_else(|| "-".to_string()),
    );
    ExitCode::SUCCESS
}
