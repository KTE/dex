//! F6's privileged sibling: reconciles `/boot/firmware/cmdline.txt`'s
//! `video=<connector>:<mode>` token with `/etc/dex/exhibit.json`'s
//! `kms_force`, idempotently.
//!
//! WHY A SEPARATE BINARY. `dex-loop` runs as the unprivileged `dex` user
//! under `ProtectSystem=strict` and must never write boot config -- that
//! sandbox is a design feature (see deploy/dex-loop.service), not an
//! oversight to work around here. Deploys in this project are manual (see
//! README.md), so an operator runs this by hand, as root, after editing
//! `/etc/dex/exhibit.json` -- the same "remaining manual step" pattern the
//! systemd unit's own comment already documents for `set-default
//! multi-user.target`.
//!
//! All the actual logic -- the grammar, and the rewrite itself
//! (`reconcile_cmdline`) -- lives in `dex_loop::exhibit` and is Mac-testable
//! with no root and no real `/boot`. This binary is a thin, privileged shell
//! around it, exactly the split `src/main.rs` itself follows for the player.
//!
//! Usage: sudo dex-exhibit-apply [--exhibit-config PATH] [--cmdline-path PATH]
//!   exit 0  -- cmdline.txt reconciled, or already correct ("no change").
//!              Prints REBOOT REQUIRED iff it actually changed.
//!   exit 1  -- cannot read or write a file (I/O failure, not a bad config).
//!   exit 2  -- refused: not running as root, an invalid exhibit config, or a
//!              rewrite that would produce an empty/multi-line cmdline.txt.

use dex_loop::exhibit::{reconcile_cmdline, ExhibitConfig, DEFAULT_EXHIBIT_CONFIG_PATH};

use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CMDLINE_PATH: &str = "/boot/firmware/cmdline.txt";

fn usage() -> ! {
    eprintln!(
        "usage: dex-exhibit-apply [--exhibit-config PATH] [--cmdline-path PATH]

Reconciles the kernel cmdline's video=<connector>:<mode> token with
/etc/dex/exhibit.json's kms_force (F6), idempotently -- every other token,
its order, and every OTHER connector's video= token are preserved untouched.
Writes one timestamped backup before any change. Must run as root.

  --exhibit-config PATH   default: {DEFAULT_EXHIBIT_CONFIG_PATH}
  --cmdline-path PATH     default: {DEFAULT_CMDLINE_PATH}   (test/bench override)

Run this after editing /etc/dex/exhibit.json. It prints REBOOT REQUIRED iff
cmdline.txt actually changed -- dex-loop binds the display from the RUNNING
kernel's /proc/cmdline, not from this file on disk, so an unrebooted change
has no effect yet and the next start's cmdline gate will say so."
    );
    std::process::exit(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut exhibit_config_path = DEFAULT_EXHIBIT_CONFIG_PATH.to_string();
    let mut cmdline_path = DEFAULT_CMDLINE_PATH.to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--exhibit-config" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                exhibit_config_path = v.clone();
            }
            "--cmdline-path" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                cmdline_path = v.clone();
            }
            "-h" | "--help" => usage(),
            _ => usage(),
        }
        i += 1;
    }

    // Refused before touching any file: "run this as root" is a clean, whole
    // failure, rather than a confusing partial write that dies on the second
    // fs::write with a permission error.
    if !is_root() {
        eprintln!("error: dex-exhibit-apply must run as root (sudo dex-exhibit-apply)");
        return ExitCode::from(2);
    }

    let config_text = match fs::read_to_string(&exhibit_config_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {exhibit_config_path}: {e}");
            return ExitCode::from(1);
        }
    };
    let config = match ExhibitConfig::from_json(&config_text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {exhibit_config_path}: {e}");
            return ExitCode::from(2);
        }
    };

    let current = match fs::read_to_string(&cmdline_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {cmdline_path}: {e}");
            return ExitCode::from(1);
        }
    };

    let desired = match reconcile_cmdline(&current, &config.connector, &config.kms_force) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let current_trimmed = current.trim_end_matches(['\n', '\r']);
    if current_trimmed == desired {
        println!("dex-exhibit-apply: {cmdline_path} already matches the exhibit config -- no change");
        return ExitCode::SUCCESS;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_path = format!("{cmdline_path}.bak-{stamp}");
    if let Err(e) = fs::write(&backup_path, &current) {
        eprintln!("error: cannot write backup {backup_path}: {e}");
        return ExitCode::from(1);
    }

    // Preserve the file's own trailing-newline convention rather than impose
    // one -- cmdline.txt is conventionally a single line with NO trailing
    // newline, but writing back exactly what was there (minus the one token
    // this tool changed) is the more conservative move regardless of which
    // convention a given card image happens to use.
    let to_write = if current.ends_with('\n') {
        format!("{desired}\n")
    } else {
        desired.clone()
    };
    if let Err(e) = fs::write(&cmdline_path, &to_write) {
        eprintln!("error: cannot write {cmdline_path}: {e}");
        return ExitCode::from(1);
    }

    println!("dex-exhibit-apply: {cmdline_path}");
    println!("  old: {current_trimmed}");
    println!("  new: {desired}");
    println!("  backup: {backup_path}");
    println!(
        "REBOOT REQUIRED -- dex-loop binds the display from the RUNNING kernel's /proc/cmdline"
    );

    ExitCode::SUCCESS
}

#[cfg(unix)]
fn is_root() -> bool {
    // geteuid() has no safe std wrapper, and this crate deliberately keeps no
    // libc dependency (Cargo.toml's SPEC §5c policy) for one syscall — the
    // same reasoning that keeps main.rs's mpv bindings hand-written rather
    // than pulled in via a crate. The unsafety is exactly this one call.
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}
