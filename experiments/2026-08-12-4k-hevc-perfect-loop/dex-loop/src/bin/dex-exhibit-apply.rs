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
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CMDLINE_PATH: &str = "/boot/firmware/cmdline.txt";

/// How many `cmdline.txt.bak-*` backups to keep. The boot partition is a
/// small FAT32 volume; unbounded accumulation would eventually fill it, and
/// a backup older than the last few applies has no recovery value anyway.
const BACKUPS_TO_KEEP: usize = 5;

/// Write `contents` to `path` and fsync it before returning. A plain
/// `fs::write` leaves the data in the page cache with no durability
/// guarantee -- on the one file the Pi cannot boot without, for an
/// installation whose documented off-switch is the mains, "written" must
/// mean "on the card", not "scheduled".
fn write_synced(path: &str, contents: &[u8]) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(contents)?;
    f.sync_all()
}

/// Best-effort fsync of `path`'s parent directory, so the rename that put
/// the file there is itself committed. Errors are deliberately ignored:
/// by this point the data blocks and the file are already synced, and some
/// filesystems refuse directory fsync -- failing the whole apply over the
/// least important of the three syncs would be worse than proceeding.
fn sync_parent_dir(path: &str) {
    let parent = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// Pick a backup path that does not already exist: two applies within the
/// same second must not silently truncate each other's backup.
fn fresh_backup_path(cmdline_path: &str, stamp: u64) -> String {
    let base = format!("{cmdline_path}.bak-{stamp}");
    let mut candidate = base.clone();
    let mut n = 1u32;
    while Path::new(&candidate).exists() {
        candidate = format!("{base}.{n}");
        n += 1;
    }
    candidate
}

/// Delete all but the newest [`BACKUPS_TO_KEEP`] `<cmdline>.bak-*` files.
/// Best-effort and loud about what it removes; a failure here never fails
/// the apply (the reconcile already succeeded), it only means one extra
/// backup survives until the next run.
///
/// Ordered by modification time, NOT by name, and `just_written` is never a
/// prune candidate at all. Both matter for the same reason, caught live on
/// the bench (dexpi4, 2026-08-17): once pruning frees an unsuffixed
/// `bak-<stamp>` name, a later same-second apply reuses it — and that name
/// sorts lexically BEFORE its older `.1`/`.2` siblings, so a name sort would
/// classify the NEWEST backup as oldest and delete the one backup that
/// still matches the file just replaced. FAT mtime granularity (2 s) can
/// still tie same-second backups, which the explicit `just_written`
/// exclusion makes harmless.
fn prune_old_backups(cmdline_path: &str, just_written: &str) {
    let path = Path::new(cmdline_path);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(if dir.as_os_str().is_empty() { Path::new(".") } else { dir })
    else {
        return;
    };
    let mut backups: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with(&prefix) {
                return None;
            }
            let p = e.path().to_string_lossy().into_owned();
            if p == just_written {
                return None;
            }
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
            Some((mtime, p))
        })
        .collect();
    // just_written is excluded above but still counts toward the kept total.
    let keep_others = BACKUPS_TO_KEEP.saturating_sub(1);
    if backups.len() <= keep_others {
        return;
    }
    backups.sort();
    let excess = backups.len() - keep_others;
    for (_, old) in backups.into_iter().take(excess) {
        match fs::remove_file(&old) {
            Ok(()) => println!("  pruned old backup: {old}"),
            Err(e) => eprintln!("warning: could not prune old backup {old}: {e}"),
        }
    }
}

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
    let backup_path = fresh_backup_path(&cmdline_path, stamp);
    // Synced before the original is touched: a backup that is still only in
    // the page cache when the mains go off is no backup at all.
    if let Err(e) = write_synced(&backup_path, current.as_bytes()) {
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
    // NEVER rewrite cmdline.txt in place. `fs::write` is open(O_TRUNC) +
    // write: a mains cut between the truncate and the data commit leaves a
    // zero-length or garbage cmdline.txt -- an unbootable Pi in a gallery
    // with no operator, recoverable only by pulling the SD card on site.
    // And this tool's next printed word is "REBOOT REQUIRED", i.e. it
    // actively invites a power cycle while an unsynced write could still be
    // sitting in the page cache. So: write a sibling temp file, fsync it,
    // rename over the original, fsync the directory. Even where FAT32's
    // rename atomicity is weak, temp+sync+rename strictly shrinks the
    // corruption window versus in-place truncation.
    let tmp_path = format!("{cmdline_path}.new");
    if let Err(e) = write_synced(&tmp_path, to_write.as_bytes()) {
        eprintln!("error: cannot write {tmp_path}: {e}");
        return ExitCode::from(1);
    }
    if let Err(e) = fs::rename(&tmp_path, &cmdline_path) {
        eprintln!("error: cannot rename {tmp_path} over {cmdline_path}: {e}");
        // Best-effort cleanup; the original is untouched either way.
        let _ = fs::remove_file(&tmp_path);
        return ExitCode::from(1);
    }
    sync_parent_dir(&cmdline_path);

    println!("dex-exhibit-apply: {cmdline_path}");
    println!("  old: {current_trimmed}");
    println!("  new: {desired}");
    println!("  backup: {backup_path}");
    prune_old_backups(&cmdline_path, &backup_path);
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
