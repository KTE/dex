//! Round-trip checker for dex-loop's F3 sidecar contract.
//!
//! Validates a sidecar by running the player's OWN parser and hash
//! (`Sidecar::from_json`, `verify_payload`) rather than a bash/jq
//! reimplementation of the grammar that could silently drift from it. Used by
//! scripts/make-sidecar.sh on every sidecar it writes or checks.
//!
//! WHY THIS IS A CARGO BIN AND NOT A STANDALONE FILE: it used to `#[path]`
//! include ../dex-loop/src/{sidecar,sha256}.rs and build under plain
//! `rustc`. Those modules now use serde_json and sha2 (SPEC §5c), which
//! `rustc` alone cannot resolve — so the checker joined the crate rather than
//! give up the property that makes it worth having. It uses only the pure
//! library (no libmpv, no DRM), so it still builds and runs on the Mac exactly
//! as on the Pi.
//!
//! Build: cargo build --release --bin sidecar-check
//! Usage: sidecar-check <sidecar.json> <stream-file>
//!   exit 0, "OK ..." on stdout   -- sidecar parses AND its sha256 matches
//!                                   the stream's exact on-disk bytes
//!   exit 1, error on stderr     -- whatever the real parser/verifier said
//!   exit 2                      -- usage error

use dex_loop::sidecar;

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
