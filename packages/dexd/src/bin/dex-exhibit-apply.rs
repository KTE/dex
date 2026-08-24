//! Reconciles the `video=<connector>:<mode>` option in
//! `/boot/firmware/cmdline.txt` with the exhibit config's `kms_force`,
//! idempotently. An operator runs it as root after editing the config, because
//! dexd runs unprivileged under `ProtectSystem=strict` and never writes boot
//! config. See docs/design/exhibit-config.md#dex-exhibit-apply.
//!
//! The grammar and the rewrite are `reconcile_cmdline` in `dexd::exhibit`,
//! which needs neither root nor a real `/boot`; this binary is the privileged
//! wrapper around it.
//!
//! Usage: `sudo dex-exhibit-apply [--exhibit-config <path>] [--cmdline-path <path>]`
//!
//! Exit codes:
//! * 0 — cmdline.txt reconciled, or already correct ("no change").
//!   `REBOOT REQUIRED` is printed only when the file changed.
//! * 1 — a file could not be read or written.
//! * 2 — refused: the process is not root, the exhibit config is invalid, or
//!   the rewrite would produce an empty or multi-line cmdline.txt.

use dexd::exhibit::{
    load_exhibit_config, reconcile_cmdline, DEFAULT_EXHIBIT_CONFIG_PATH,
    DEFAULT_EXHIBIT_CONFIG_PATHS,
};

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CMDLINE_PATH: &str = "/boot/firmware/cmdline.txt";

/// How many `cmdline.txt.bak-*` backups to keep. The boot partition is a small
/// FAT32 volume, so the count is capped.
const BACKUPS_TO_KEEP: usize = 5;

/// Write `contents` to `path` and fsync it before returning. A plain
/// `fs::write` leaves the data in the page cache, and this is the one file the
/// Raspberry Pi cannot boot without, on an installation whose power is cut
/// without a shutdown.
///
/// See docs/design/exhibit-config.md#dex-exhibit-apply.
fn write_synced(path: &str, contents: &[u8]) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(contents)?;
    f.sync_all()
}

/// Fsync `path`'s parent directory, so the rename that put the file there is
/// committed too. Errors are ignored: the data blocks and the file are already
/// synced by this point, and some filesystems refuse a directory fsync.
fn sync_parent_dir(path: &str) {
    let parent = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// Pick a backup path that does not already exist, so two applies within the
/// same second cannot truncate each other's backup.
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

/// Delete all but the newest [`BACKUPS_TO_KEEP`] `<cmdline>.bak-*` files, and
/// print what it removes. A failure here leaves one extra backup until the next
/// run and never fails an apply that has already succeeded.
///
/// Keep two properties when changing this: order the candidates by modification
/// time, never by name, and leave `just_written` out of the candidate list.
/// Dropping either one deletes the newest backup instead of the oldest.
/// See docs/design/exhibit-config.md#dex-exhibit-apply.
fn prune_old_backups(cmdline_path: &str, just_written: &str) {
    let path = Path::new(cmdline_path);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    }) else {
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
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
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
        "usage: dex-exhibit-apply [--exhibit-config <path>] [--cmdline-path <path>]

Reconciles the kernel command line's video=<connector>:<mode> option with the
exhibit config's kms_force, idempotently. Every other option, its position, and
every other connector's video= option stay as they are. Writes one timestamped
backup before any change. Must run as root.

  --exhibit-config <path>
                          default: whichever of {DEFAULT_EXHIBIT_CONFIG_PATHS:?}
                          exists (at most one may; the file extension selects
                          the parser -- .json is strict JSON, .yaml is YAML)
  --cmdline-path <path>   default: {DEFAULT_CMDLINE_PATH}   (test rig only)

Run this after editing the exhibit config. It prints REBOOT REQUIRED only when
cmdline.txt changed. dexd binds the display from the running kernel's
/proc/cmdline, so an unrebooted change has no effect yet and the next start's
cmdline check reports the mismatch."
    );
    std::process::exit(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    // `None` asks for discovery, so this tool searches the same .json/.yaml
    // names in the same order dexd does.
    let mut exhibit_config_path: Option<String> = None;
    let mut cmdline_path = DEFAULT_CMDLINE_PATH.to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--exhibit-config" => {
                i += 1;
                let Some(v) = args.get(i) else { usage() };
                exhibit_config_path = Some(v.clone());
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

    // Checked before any file is touched, so a non-root run fails whole and
    // early, instead of dying half-way through on a permission error.
    if !is_root() {
        eprintln!("error: dex-exhibit-apply must run as root (sudo dex-exhibit-apply)");
        return ExitCode::from(2);
    }

    // The same loader dexd uses, including its .json/.yaml discovery and its
    // refusal when both files exist: reading a different file than the player
    // reads would make the boot config and the player's config diverge.
    // See docs/design/exhibit-config.md#dex-exhibit-apply.
    let (config, exhibit_config_path) = match load_exhibit_config(
        exhibit_config_path.as_deref(),
        &DEFAULT_EXHIBIT_CONFIG_PATHS,
    ) {
        Ok(Some(found)) => found,
        // Without a config this tool has nothing to do, so a missing config is
        // refused here.
        Ok(None) => {
            eprintln!(
                "error: no exhibit config found (looked for {}). Create \
                     {DEFAULT_EXHIBIT_CONFIG_PATH} next to the video, or name a config with \
                     --exhibit-config",
                DEFAULT_EXHIBIT_CONFIG_PATHS.join(", ")
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
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

    // Name the config this run read before reporting what it did: with two
    // possible names and an override flag, that is the first thing to check
    // when the display mode is wrong. See dex-exhibit-apply(1).
    println!(
        "dex-exhibit-apply: applying {exhibit_config_path} (connector {}, kms_force {})",
        config.connector, config.kms_force
    );

    let desired = match reconcile_cmdline(&current, &config.connector, &config.kms_force) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let current_trimmed = current.trim_end_matches(['\n', '\r']);
    if current_trimmed == desired {
        println!(
            "dex-exhibit-apply: {cmdline_path} already matches the exhibit config -- no change"
        );
        return ExitCode::SUCCESS;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_path = fresh_backup_path(&cmdline_path, stamp);
    // Synced before the original is touched, so a power cut cannot leave the
    // backup unwritten.
    if let Err(e) = write_synced(&backup_path, current.as_bytes()) {
        eprintln!("error: cannot write backup {backup_path}: {e}");
        return ExitCode::from(1);
    }

    // Keep the file's own trailing-newline convention. cmdline.txt is
    // conventionally a single line with no trailing newline, but card images
    // vary, so write back what was there, minus the one option this tool
    // changed.
    let to_write = if current.ends_with('\n') {
        format!("{desired}\n")
    } else {
        desired.clone()
    };
    // Write a sibling temp file, fsync it, rename it over the original, then
    // fsync the directory. Do not rewrite cmdline.txt in place: `fs::write`
    // opens with O_TRUNC, so a power cut between the truncate and the data
    // commit leaves a Raspberry Pi that will not boot.
    // See docs/design/exhibit-config.md#dex-exhibit-apply.
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
    println!("REBOOT REQUIRED -- dexd binds the display from the running kernel's /proc/cmdline");

    ExitCode::SUCCESS
}

#[cfg(unix)]
fn is_root() -> bool {
    // `geteuid` has no safe wrapper in std, and the crate declares no libc
    // dependency for one syscall. This call is the only unsafe code in this
    // binary.
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}
