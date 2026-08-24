//! Failure-path tests. Each spawns the real binary and asserts the exit code
//! and that the run ends at all.
//!
//! The device under test may be showing something, so every invocation here
//! that can reach `mpv_create` carries `--no-defaults --opt vo=null --opt
//! vid=no --opt aid=no`: the null video output never touches DRM, and
//! deselecting every track makes mpv end on its own (`NOTHING_TO_PLAY` →
//! `END_FILE`). One test departs from the rule and carries `#[ignore]`.
//!
//! The binary links libmpv, so this target runs where libmpv is installed;
//! elsewhere use `cargo test --lib`.
//!
//! See docs/design/development.md#display-safety and
//! docs/design/development.md#test-harness.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Exit 2 is a refusal before playback; exit 1 is a runtime failure.
/// See docs/design/startup-checks.md#exit-codes.
const REFUSED_EXIT: i32 = 2;
const RUNTIME_EXIT: i32 = 1;

/// Outcome of one run. `exit_code` is `None` both when the harness killed the
/// child at its deadline and when the child died by signal, so a test whose
/// pass condition is survival reads `deadline_killed`, which is true only for
/// the harness's own kill.
/// See docs/design/development.md#test-harness.
struct Run {
    exit_code: Option<i32>,
    deadline_killed: bool,
    stderr: String,
}

/// Spawn dexd, wait at most `deadline`, and kill the child on overrun. The
/// deadline is the assertion: it is what catches a player that hangs on a
/// failure path instead of exiting.
/// See docs/design/development.md#test-harness.
fn run_with_deadline(args: &[&str], deadline: Duration) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dexd"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dexd");
    // Drain stderr on a thread so a chatty child cannot fill the pipe and
    // block, which would look like a hang.
    let mut pipe = child.stderr.take().expect("stderr piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        pipe.read_to_string(&mut s).ok();
        s
    });
    let start = Instant::now();
    let (exit_code, deadline_killed) = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break (status.code(), false),
            None if start.elapsed() > deadline => {
                child.kill().ok();
                child.wait().ok();
                break (None, true);
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stderr = reader.join().expect("stderr reader");
    Run {
        exit_code,
        deadline_killed,
        stderr,
    }
}

/// Unique-per-test scratch path (std::env::temp_dir; no cleanup needed).
fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("dexd-test-{}-{}", std::process::id(), name));
    p
}

/// The NAL units of a real single-frame HEVC encode: parameter sets and one
/// IDR slice, taken from a 16×16 black frame with a closed GOP.
///
/// The bytes have to be a parseable stream: `loop://` never
/// returns end of file, and the demuxer's probe gives up early only when it
/// reaches one, so filler leaves it demanding data until the deadline kills
/// the run. Real parameter sets let it resolve width, height and profile in
/// one pass, after which mpv reaches `END_FILE` in well under a second.
/// Nothing here has to decode: video is deselected before decode is attempted.
/// See docs/design/development.md#fixtures.
fn stub_annexb() -> Vec<u8> {
    fn nal(nal_type: u8, payload: &[u8]) -> Vec<u8> {
        // 4-byte start code + 2-byte NAL header (forbidden=0, layer=0, tid+1=1)
        let mut v = vec![0, 0, 0, 1, nal_type << 1, 0x01];
        v.extend_from_slice(payload);
        v
    }
    let mut v = Vec::new();
    v.extend(nal(
        32,
        &[
            0x0c, 0x01, 0xff, 0xff, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0x9f, 0xa8, 0x00, 0x00,
            0x03, 0x00, 0x00, 0x1e, 0xba, 0x02, 0x40,
        ],
    )); // VPS
    v.extend(nal(
        33,
        &[
            0x01, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0x9f, 0xa8, 0x00, 0x00, 0x03, 0x00, 0x00,
            0x1e, 0xa0, 0x88, 0x45, 0x96, 0xea, 0xaf, 0x2b, 0xc0, 0x5a, 0x02, 0x00, 0x00, 0x03,
            0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x02, 0x10,
        ],
    )); // SPS
    v.extend(nal(34, &[0xc1, 0x73, 0xc0, 0x89])); // PPS
    v.extend(nal(
        20,
        &[0xaf, 0x78, 0xf8, 0x5d, 0xf7, 0xff, 0xe2, 0xc0, 0x38],
    )); // IDR_N_LP slice
    v
}

/// Write the sidecar `<asset>.json` binding `bytes` at `fps`, as a normal
/// start requires. See docs/design/sidecar.md#rate-and-identity.
fn write_sidecar(asset: &Path, bytes: &[u8], fps: &str) {
    let sha = dexd::sha256::sha256_hex(bytes);
    std::fs::write(
        format!("{}.json", asset.display()),
        format!(r#"{{"fps":"{fps}","sha256":"{sha}"}}"#),
    )
    .unwrap();
}

/// Write an exhibit config that every display check accepts, at a unique temp
/// path. `display_mode: "auto"` names no resolution, so the mode pre-flight is
/// skipped, and `kms_force: "none"` pairs with [`write_no_video_cmdline`] so
/// the cmdline check passes whatever the host's own kernel command line
/// carries. Tests that want a different check to refuse pass both files, so
/// only the one under test can fail.
/// See docs/design/startup-checks.md#order-of-checks.
fn write_exhibit_config(name: &str) -> PathBuf {
    let p = temp_path(name);
    std::fs::write(&p, r#"{"display_mode":"auto","kms_force":"none"}"#).unwrap();
    p
}

/// Write a kernel command line with no `video=` token, so the cmdline check's
/// "kms_force=none, expect no token" branch matches on any host. A container
/// carries no such token and a configured Raspberry Pi does, so a test that
/// read the real file would assert different things on each.
/// See docs/design/development.md#test-only-flags.
fn write_no_video_cmdline(name: &str) -> PathBuf {
    let p = temp_path(name);
    std::fs::write(&p, "console=ttyS0 root=/dev/mmcblk0p2 rootwait quiet\n").unwrap();
    p
}

#[test]
fn missing_file_exits_2_and_names_the_path() {
    let cfg = write_exhibit_config("missingfile-exhibit.json");
    let cl = write_no_video_cmdline("missingfile-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/dexd-test.265",
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("/nonexistent/dexd-test.265"),
        "stderr must name the path: {}",
        r.stderr
    );
}

#[test]
fn empty_file_exits_2() {
    let p = temp_path("empty.265");
    std::fs::write(&p, b"").unwrap();
    let cfg = write_exhibit_config("emptyfile-exhibit.json");
    let cl = write_no_video_cmdline("emptyfile-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    // Name the message, not only the code: the missing-sidecar path also
    // exits 2, so a bare code check would stay green if the empty-payload
    // check were deleted.
    assert!(r.stderr.contains("is empty"), "stderr: {}", r.stderr);
}

/// A flag at the end of the command line with no value must refuse with the
/// usage text. Dropping it silently would let a line-continuation typo in a
/// unit file start the player on the connector's preferred mode, or with no
/// frame-rate cross-check, and say nothing.
/// See docs/design/startup-checks.md#argument-shape.
///
/// No test here runs dexd with no arguments at all: on a device whose config
/// and asset are complete that starts playback and takes DRM master. The
/// argument shape of the packaged unit is covered by
/// `no_positional_asset_is_accepted_and_reaches_the_config`, which supplies
/// its own `--exhibit-config` and lands on a check.
#[test]
fn fps_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(&["/nonexistent/x.265", "--fps"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

#[test]
fn mode_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(&["/nonexistent/x.265", "--mode"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// `mpv_create` enables idle mode, so a failed load emits `END_FILE` and then
/// waits, alive, unless the event loop treats that event as fatal. Deselecting
/// every track forces a display-free playback failure; the process must exit 1
/// inside the deadline.
/// See docs/design/failure-handling.md#fatal-events.
#[test]
fn playback_failure_exits_nonzero_never_hangs() {
    let p = temp_path("stub.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("playbackfail-exhibit.json");
    let cl = write_no_video_cmdline("playbackfail-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert!(
        r.exit_code.is_some(),
        "the player hangs on a playback failure (killed at the deadline); stderr: {}",
        r.stderr
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    // Four other sites also exit 1 (mpv_create, set_opt, mpv_initialize,
    // loadfile), so check the message as well. A break that made one of those
    // fail first would exit 1 at once and leave this test green without ever
    // reaching the END_FILE branch.
    assert!(
        r.stderr.contains("playback ended"),
        "exited 1 without reaching the END_FILE branch: {}",
        r.stderr
    );
}

/// Bytes that are not an HEVC stream never reach mpv: the asset check refuses
/// them at startup, naming the missing start code.
/// See docs/design/sidecar.md#the-asset-check.
#[test]
fn garbage_bytes_refused_at_startup_exit_2() {
    let p = temp_path("garbage.265");
    // 64 KiB of bytes in 0x02..=0x7E: no 0x00/0x01 -> no start code anywhere.
    let bytes: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761) >> 24) as u8 % 0x7d) + 0x02)
        .collect();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30"); // the checksum matches, so the asset check is what refuses
    let cfg = write_exhibit_config("garbage-exhibit.json");
    let cl = write_no_video_cmdline("garbage-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("start code"), "stderr: {}", r.stderr);
}

/// An asset whose first picture is a CRA — an open GOP — and whose checksum
/// matches its sidecar. The sidecar check passes and the asset check must
/// still refuse: a matching checksum says the bytes are the prepared ones, and
/// nothing about whether they can loop.
/// See docs/design/sidecar.md#the-asset-check.
#[test]
fn open_gop_asset_refused_at_startup_exit_2() {
    let p = temp_path("opengop.265");
    let mut bytes = Vec::new();
    for t in [32u8, 33, 34, 21] {
        // VPS SPS PPS CRA
        bytes.extend([0, 0, 0, 1, t << 1, 0x01]);
        bytes.extend([0x2a; 8]);
    }
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("opengop-exhibit.json");
    let cl = write_no_video_cmdline("opengop-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("CRA"), "stderr: {}", r.stderr);
}

// ---- The sidecar check ---------------------------------------------------

#[test]
fn missing_sidecar_refused_exit_2_naming_the_sidecar_path() {
    let p = temp_path("nosidecar.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let _ = std::fs::remove_file(format!("{}.json", p.display()));
    let cfg = write_exhibit_config("missingsidecar-exhibit.json");
    let cl = write_no_video_cmdline("missingsidecar-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains(&format!("{}.json", p.display())),
        "stderr must name the sidecar path: {}",
        r.stderr
    );
}

/// A half-copied asset: the leading NAL units are intact, so the asset check
/// passes, and only the sidecar's checksum sees the missing tail.
/// See docs/design/sidecar.md#checksum-verification.
#[test]
fn truncated_asset_vs_full_hash_refused_exit_2() {
    let p = temp_path("truncated.265");
    let full = stub_annexb();
    write_sidecar(&p, &full, "30");
    std::fs::write(&p, &full[..full.len() - 20]).unwrap();
    let cfg = write_exhibit_config("truncated-exhibit.json");
    let cl = write_no_video_cmdline("truncated-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("sha256"), "stderr: {}", r.stderr);
}

#[test]
fn fps_contradicting_sidecar_refused_exit_2_naming_both() {
    let p = temp_path("fpsconflict.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("fpsconflict-exhibit.json");
    let cl = write_no_video_cmdline("fpsconflict-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "25",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("25") && r.stderr.contains("30"),
        "stderr must name both rates: {}",
        r.stderr
    );
}

/// Reaching mpv, and so exit 1, is what proves the checks passed when `--fps`
/// agrees with the sidecar.
#[test]
fn agreeing_fps_and_sidecar_reach_playback() {
    let p = temp_path("fpsagree.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("fpsagree-exhibit.json");
    let cl = write_no_video_cmdline("fpsagree-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    // Disambiguate from the other four exit-1 sites -- see the comment on
    // playback_failure_exits_nonzero_never_hangs.
    assert!(r.stderr.contains("playback ended"), "stderr: {}", r.stderr);
}

/// An option libmpv rejects exits 2: the same asset and flags fail identically
/// on every restart, so it names an invocation to fix; a restart cannot clear
/// it.
/// See docs/design/startup-checks.md#exit-codes.
#[test]
fn bad_opt_value_refused_exit_2_not_1() {
    let p = temp_path("badopt.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("badopt-exhibit.json");
    let cl = write_no_video_cmdline("badopt-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
            "--opt",
            "this-option-does-not-exist=1",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
}

/// A frame rate written as a fraction reaches mpv's `container-fps-override`
/// and is accepted, so an asset at 30000/1001 frames per second plays instead
/// of leaving the device in a restart loop.
/// See docs/design/sidecar.md#frame-rate-resolution.
#[test]
fn rational_fps_reaches_playback() {
    let p = temp_path("ntsc.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // the override needs no sidecar
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30000/1001",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("playback ended"), "stderr: {}", r.stderr);
}

/// The `--test-rig-no-sidecar --fps` override (test rig only) skips the
/// sidecar check and reaches playback with no sidecar on disk: exit 1, not 2.
/// See docs/design/startup-checks.md#test-rig-only-override.
#[test]
fn test_rig_override_bypasses_the_sidecar_check() {
    let p = temp_path("override.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // no sidecar written
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    // Disambiguate from the other four exit-1 sites -- see the comment on
    // playback_failure_exits_nonzero_never_hangs.
    assert!(r.stderr.contains("playback ended"), "stderr: {}", r.stderr);
}

/// The override is two flags or nothing: `--test-rig-no-sidecar` without
/// `--fps` is refused, so a frame rate can never fall back to the command line
/// on a deployment. See docs/design/startup-checks.md#test-rig-only-override.
#[test]
fn test_rig_override_without_fps_refused_exit_2() {
    let p = temp_path("override-nofps.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
}

// ---- Build identity and heartbeat ----------------------------------------

#[test]
fn startup_identifies_version_and_build() {
    // Even a refused start prints its version and commit, so a log that opens
    // with this process can still be read weeks later.
    // See docs/design/startup-checks.md#startup-lines.
    let r = run_with_deadline(&["/nonexistent/x.265"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(REFUSED_EXIT));
    assert!(
        r.stderr
            .contains(&format!("dexd {}", env!("CARGO_PKG_VERSION"))),
        "stderr: {}",
        r.stderr
    );
}

// ---- The health check ----------------------------------------------------

/// dexd registers `mpv_observe_property("time-pos", ...)` whenever
/// `mpv_initialize` succeeds, and the registration must be silent. An FFI slip
/// that made it fail without crashing would leave in-place recovery switched
/// off for the run with only one warning line to show for it.
///
/// The registration is the whole of what this covers. A stall, a recovery and
/// an escalation need real decode, which this file's option rule keeps out;
/// the policy behind them is tested in `src/health.rs`.
/// See docs/design/failure-handling.md#health-check.
#[test]
fn health_check_registers_without_warning_during_normal_playback() {
    let p = temp_path("healthreg.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("healthreg-exhibit.json");
    let cl = write_no_video_cmdline("healthreg-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(
        !r.stderr.contains("mpv_observe_property"),
        "the time-pos subscription failed to register: {}",
        r.stderr
    );
}

#[test]
fn heartbeat_zero_is_emitted_at_startup() {
    // The 10-minute cadence is out of reach for a test. Heartbeat 0 is emitted
    // right after loadfile and covers the temperature reading and the line
    // format. It says nothing about the property subscriptions: at heartbeat 0
    // the frame-drop, delay and position fields read "n/a", because nothing
    // has decoded yet. See docs/design/failure-handling.md#heartbeat.
    let p = temp_path("heartbeat.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let cfg = write_exhibit_config("heartbeat-exhibit.json");
    let cl = write_no_video_cmdline("heartbeat-cmdline");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("heartbeat loops="),
        "stderr: {}",
        r.stderr
    );
}

// ---- Forced in-place recovery (test rig only) -----------------------------
//
// `--test-rig-force-recovery-after-secs` forces the same recovery decision an
// organic stall produces, on a timer, during otherwise-healthy playback. The
// decision and the trigger are `HealthMonitor::force_recovery` and
// `ForceRecoveryTrigger` in dexd::health; main.rs routes both the organic tick
// and the probe through `act_on_health_action`, so the probe drives the same
// mpv-facing mechanics.
//
// The tests here keep this file's `--opt vid=no --opt aid=no` rule, so they
// cover the command-line half: the flag parses, it is refused without
// `--test-rig-no-sidecar`, and the arming warning prints. Firing needs a
// health-check tick against playback that is still alive, and those options
// make mpv reach "nothing to play" in well under a second. The one test that
// lets the probe fire carries `#[ignore]`.
// See docs/design/failure-handling.md#test-rig-probes.

/// `--test-rig-force-recovery-after-secs` without `--test-rig-no-sidecar` is
/// refused on the shape of the command line alone, before the asset path is
/// read. The packaged unit passes a real sidecar and never passes
/// `--test-rig-no-sidecar`, so the probe cannot arm against a deployed asset
/// however the flag reached the command line.
/// See docs/design/startup-checks.md#test-rig-only-override.
#[test]
fn force_recovery_without_test_rig_no_sidecar_refused_exit_2() {
    let r = run_with_deadline(
        &[
            "/nonexistent/x.265",
            "--test-rig-force-recovery-after-secs",
            "5",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("--test-rig-no-sidecar"),
        "stderr must explain the required pairing: {}",
        r.stderr
    );
}

/// Same missing-value rule as `--fps` and `--mode`: a flag at the end of the
/// command line with no value refuses with the usage text.
#[test]
fn force_recovery_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--test-rig-force-recovery-after-secs"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// A non-numeric value refuses with the usage text; it does not parse as 0 and
/// does not panic the process.
#[test]
fn force_recovery_flag_non_numeric_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &[
            "/nonexistent/x.265",
            "--test-rig-force-recovery-after-secs",
            "soon",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// Paired with `--test-rig-no-sidecar`, the flag arms: startup prints the
/// warning naming it, and playback proceeds as it does without the flag. `N`
/// is large enough that the trigger cannot fire before the run's own fast exit
/// via "nothing to play", so this test covers the wiring and the warning; the
/// recovery itself is covered below.
/// See docs/design/failure-handling.md#test-rig-probes.
#[test]
fn force_recovery_flag_with_test_rig_no_sidecar_arms_and_reaches_playback() {
    let p = temp_path("forcerecovery.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // the override needs no sidecar
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30",
            "--test-rig-force-recovery-after-secs",
            "3600",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("playback ended"), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("(test rig only)")
            && r.stderr
                .contains("--test-rig-force-recovery-after-secs=3600 is armed"),
        "startup must warn that the probe is armed: {}",
        r.stderr
    );
    // The trigger must not have fired in a run this short: it would have fired
    // against a process already past "nothing to play", which is a different
    // scenario from the one this test covers.
    assert!(
        !r.stderr
            .contains("--test-rig-force-recovery-after-secs elapsed"),
        "the trigger must not have had a chance to fire here: {}",
        r.stderr
    );
}

/// Force an in-place recovery a few seconds into healthy playback against a
/// real mpv, and assert the process survives it: the recovery's own
/// `END_FILE(reason=stop)` is absorbed and the process is still running when
/// the harness kills it at the deadline.
///
/// This is the one test here that keeps the video track selected, so the
/// position advances and a health-check tick can read healthy playback before
/// the trigger fires; `--opt vo=null` keeps it headless, and only the deadline
/// bounds the decode. Hence `#[ignore]`: it runs by exact name, in a CI step
/// of its own, and on a Raspberry Pi only with a free display and no
/// long-running test in progress. It covers survival under software decode;
/// the picture after a recovery needs the hardware-decode options this run
/// skips, so that half rests on a manual run against a real display.
///
/// ```text
/// cargo test --test cli force_recovery_survives_against_real_mpv -- --ignored --nocapture
/// ```
///
/// See docs/design/development.md#display-safety and
/// docs/design/failure-handling.md#test-rig-probes.
#[test]
#[ignore]
fn force_recovery_survives_against_real_mpv() {
    let p = temp_path("liverecovery.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30",
            "--test-rig-force-recovery-after-secs",
            "3",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    // Survival is read from `deadline_killed`, not from `exit_code == None`:
    // a child that died by signal also has no exit code, and a recovery that
    // crashed after printing the absorb line would pass the weaker check.
    assert!(
        r.deadline_killed,
        "the process must survive the forced recovery and still be running when \
         killed at the deadline; it ended on its own with exit_code {:?} -- an \
         exit means the recovery's own END_FILE(reason=stop) was not absorbed, \
         and no exit code here means death by signal. stderr: {}",
        r.exit_code, r.stderr
    );
    assert!(
        r.stderr.contains("attempting in-place recovery 1/"),
        "forced recovery never fired: {}",
        r.stderr
    );
    assert!(
        r.stderr
            .contains("absorbing the expected END_FILE(reason=stop)"),
        "the recovery's own END_FILE was not absorbed: {}",
        r.stderr
    );
    // The assertions above are also satisfied by a process that survives with
    // an event loop that stopped iterating. A recovery that worked lets the
    // position resume advancing, which feeds the health check a healthy sample
    // and means no second recovery fires. With the trigger at 3 s and ticks
    // about every 10 s, an alive-but-unresponsive process would reach a second
    // attempt around 23 s, inside this 30 s deadline -- so keep the deadline
    // above the second attempt, or this check passes with nothing running.
    // See docs/design/failure-handling.md#residual-gaps.
    assert!(
        !r.stderr.contains("attempting in-place recovery 2/"),
        "a second recovery fired after the forced one, so the position likely \
         never resumed advancing: {}",
        r.stderr
    );
}

// ---- The exhibit config ---------------------------------------------------
//
// The decisions behind these checks -- the display-mode grammar,
// `resolve_display`, the cmdline comparator, `reconcile_cmdline` and the
// pre-flight parser -- are pure functions with unit tests in src/exhibit.rs.
// The tests here cover what those cannot: the order of the checks, the parsing
// of the flags, and the refusal messages. All of them keep this file's
// `--opt vo=null --opt vid=no --opt aid=no` rule.
// See docs/design/startup-checks.md#test-scope.

/// An `--exhibit-config` naming a file that is not there is refused before the
/// asset path is read, and the refusal names the path that was given.
///
/// Naming that path is the point: the message telling an operator to create
/// the installed default would send them to a file the run they are debugging
/// does not read. See docs/design/startup-checks.md#message-rules.
#[test]
fn missing_exhibit_config_refused_exit_2_before_the_asset_is_read() {
    let improbable = temp_path("default-exhibit-config-must-not-exist.json");
    let _ = std::fs::remove_file(&improbable); // never written; just proving absence
    let r = run_with_deadline(
        &[
            "/nonexistent/missing-config.265",
            "--exhibit-config",
            improbable.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr
            .contains("default-exhibit-config-must-not-exist.json"),
        "the refusal must name the path that was actually given: {}",
        r.stderr
    );
    // The negative carries the ordering claim: the asset check must not have
    // run yet.
    assert!(
        !r.stderr.contains("missing-config.265"),
        "the exhibit check must refuse before the asset path is read, but the \
         asset-missing message appeared too: {}",
        r.stderr
    );
}

// The complementary case -- no --exhibit-config and no config in the assets
// directory, so the "no exhibit config, create this file" message fires -- is
// not tested here. Its outcome depends on whether the host has /opt/dex, so it
// would assert one thing on a workstation and another on a device. It is
// covered where it is host-independent, by
// `load_returns_none_when_no_default_exists` and `resolve_display`'s unit
// tests in src/exhibit.rs. See docs/design/startup-checks.md#test-scope.

/// A YAML exhibit config drives the binary through the same order of checks a
/// JSON one does, so the format dispatch is wired into main.rs and not only
/// unit-tested in the library.
/// See docs/design/exhibit-config.md.
#[test]
fn yaml_exhibit_config_binds_the_display_like_json_does() {
    let cfg = temp_path("yaml-exhibit.yaml");
    std::fs::write(
        &cfg,
        "# a venue would really write this\ndisplay_mode: 7680x4320@60\nkms_force: none\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("yaml-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/yaml.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    // Reaching the mode pre-flight, which refuses this 8K mode, is what shows
    // the YAML parsed, validated and bound the display: a config that failed
    // earlier could not produce this message.
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("7680x4320"), "stderr: {}", r.stderr);
}

/// The invocation the packaged unit uses: no positional asset path, everything
/// from the exhibit config. `ExecStart=/usr/bin/dexd` passes exactly this, so
/// a no-argument run refused on argument shape would leave the device in a
/// restart loop while every other test here, all of which pass arguments,
/// stayed green.
///
/// The config comes from `--exhibit-config` so the test does not depend on the
/// host having `/opt/dex`, and its mode is one no connector offers, so the run
/// lands on the mode pre-flight instead of starting playback.
/// See docs/design/service-unit.md.
#[test]
fn no_positional_asset_is_accepted_and_reaches_the_config() {
    let cfg = temp_path("noposition-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/fromconfig.265\ndisplay_mode: 7680x4320@60\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("noposition-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    // The run reached a check, and the usage text stayed unprinted.
    assert!(
        !r.stderr.contains("usage:"),
        "a no-argument run must not be refused on argument shape -- that is how \
         the packaged unit invokes the player: {}",
        r.stderr
    );
    assert!(r.stderr.contains("7680x4320"), "stderr: {}", r.stderr);
}

/// The asset comes from the config: with a valid display and a config naming
/// an asset that is not there, the run gets as far as failing to read that
/// path, which happens only if `resolve_asset` took it from the file.
#[test]
fn the_exhibit_config_names_which_asset_plays() {
    let cfg = temp_path("assetfromconfig-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/named-by-config.265\ndisplay_mode: auto\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("assetfromconfig-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("named-by-config.265"),
        "the asset named by the config must be the one the player tried to read: {}",
        r.stderr
    );
}

/// A relative `asset` names the file next to the config: the run gets as far
/// as failing to read the path formed from the config's own directory, not one
/// formed from the service's working directory. The config sits in a temp
/// directory, so the test does not depend on the host having `/opt/dex`.
/// See docs/design/exhibit-config.md.
#[test]
fn a_relative_asset_in_the_config_resolves_next_to_the_config() {
    let cfg = temp_path("relasset-exhibit.yaml");
    let artwork = temp_path("relasset-artwork.265");
    let bare = artwork.file_name().unwrap().to_str().unwrap().to_string();
    let _ = std::fs::remove_file(&artwork); // never written; the read must fail
    std::fs::write(&cfg, format!("asset: {bare}\ndisplay_mode: auto\n")).unwrap();
    let cl = write_no_video_cmdline("relasset-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains(artwork.to_str().unwrap()),
        "a bare file name must resolve against the config's directory, giving {}: {}",
        artwork.display(),
        r.stderr
    );
}

/// A positional path that contradicts the config is refused naming both, like
/// `mode_contradicting_exhibit_config_refused_naming_both` does for the mode.
#[test]
fn positional_asset_contradicting_the_config_refused_naming_both() {
    let cfg = temp_path("assetconflict-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/config-asset.265\ndisplay_mode: auto\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("assetconflict-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/cli-asset.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("config-asset.265") && r.stderr.contains("cli-asset.265"),
        "stderr must name both assets: {}",
        r.stderr
    );
}

/// A config with no `asset` and no path on the command line refuses, so no
/// file nobody named is ever played. This is the fail-closed row of
/// `resolve_asset`'s table, driven through the binary.
/// See docs/design/startup-checks.md#fail-closed-startup.
#[test]
fn no_asset_named_anywhere_refused_rather_than_guessed() {
    let cfg = temp_path("noasset-exhibit.yaml");
    std::fs::write(&cfg, "display_mode: auto\n").unwrap();
    let cl = write_no_video_cmdline("noasset-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("no asset"), "stderr: {}", r.stderr);
    // The negative: no fixed path is tried when the config names none.
    assert!(
        !r.stderr.contains("cannot read /opt/dex/artwork.265"),
        "a missing asset must never fall back to a fixed path: {}",
        r.stderr
    );
}

/// The file extension selects the parser: YAML written into a `.json` file is
/// refused instead of being accepted by a permissive parser.
/// See docs/design/exhibit-config.md.
#[test]
fn yaml_contents_in_a_json_named_config_refused() {
    let cfg = temp_path("yaml-in-json.json");
    std::fs::write(&cfg, "display_mode: 3840x2160@30\n").unwrap();
    let r = run_with_deadline(
        &[
            "/nonexistent/yamlinjson.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("JSON"), "stderr: {}", r.stderr);
}

/// An exhibit config that does not parse is refused with the parse error, kept
/// distinct from the message for a config that is missing, because the two
/// have different repairs. See docs/design/startup-checks.md#message-rules.
#[test]
fn malformed_exhibit_config_refused_exit_2_naming_the_parse_error() {
    let cfg = temp_path("malformed-exhibit.json");
    std::fs::write(&cfg, r#"{"display_mode":"auto","kms_forse":"none"}"#).unwrap(); // typo
    let r = run_with_deadline(
        &[
            "/nonexistent/malformed.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("kms_forse"), "stderr: {}", r.stderr);
}

/// `--test-rig-no-sidecar` skips the exhibit config and the cmdline check as
/// well as the sidecar: neither `--exhibit-config` nor `--proc-cmdline` is
/// given and the run still reaches playback, exit 1 instead of 2.
/// See docs/design/startup-checks.md#test-rig-only-override.
#[test]
fn test_rig_override_bypasses_the_exhibit_config_and_cmdline_checks() {
    let p = temp_path("override-nochecks.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("playback ended"), "stderr: {}", r.stderr);
}

/// A `--mode` that contradicts the exhibit config's `display_mode` is refused
/// naming both, like `fps_contradicting_sidecar_refused_exit_2_naming_both`
/// does for the frame rate.
#[test]
fn mode_contradicting_exhibit_config_refused_naming_both() {
    let cfg = temp_path("modeconflict-exhibit.json");
    std::fs::write(
        &cfg,
        r#"{"display_mode":"3840x2160@30","kms_force":"none"}"#,
    )
    .unwrap();
    let cl = write_no_video_cmdline("modeconflict-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/modeconflict.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--mode",
            "2560x1440@60",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("3840x2160@30") && r.stderr.contains("2560x1440@60"),
        "stderr must name both modes: {}",
        r.stderr
    );
}

/// The cmdline check: the exhibit config says `kms_force: none` while the
/// kernel command line still carries a `video=HDMI-A-1:...` token, which is
/// what a config edited without reconciling the boot config looks like. The
/// refusal names the repair, before the asset is read.
/// See docs/design/startup-checks.md#message-rules.
#[test]
fn cmdline_mismatch_refused_naming_dex_exhibit_apply() {
    let cfg = write_exhibit_config("cmdlinemismatch-exhibit.json"); // kms_force: none
    let cl = temp_path("cmdlinemismatch-cmdline");
    std::fs::write(&cl, "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait\n").unwrap();
    let r = run_with_deadline(
        &[
            "/nonexistent/cmdlinemismatch.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("dex-exhibit-apply") && r.stderr.contains("3840x2160@30"),
        "stderr: {}",
        r.stderr
    );
}

/// The mode pre-flight: a `display_mode` no connector offers, here 8K60, is
/// refused either because no connector is found (a container with no DRM) or
/// because the one found does not list the mode. Both messages name the
/// requested resolution, which is what this test can assert on either host.
/// See docs/design/startup-checks.md#resolution-and-refresh.
#[test]
fn implausible_mode_refused_by_the_sysfs_preflight() {
    let cfg = temp_path("implausible-exhibit.json");
    std::fs::write(
        &cfg,
        r#"{"display_mode":"7680x4320@60","kms_force":"none"}"#,
    )
    .unwrap();
    let cl = write_no_video_cmdline("implausible-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/implausible.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("7680x4320"),
        "stderr must name the implausible resolution: {}",
        r.stderr
    );
    // And not the asset-missing message: the display checks run first, as in
    // the "no exhibit config" case above.
    assert!(
        !r.stderr.contains("implausible.265"),
        "stderr: {}",
        r.stderr
    );
}

// ---- A forced supervisor-thread hang (test rig only) ----------------------
//
// An mpv core that stops responding goes silent on the supervisor thread, and
// the health check acts on that silence in-process. Neither covers the
// supervisor thread hanging in code that is not an mpv call at all, such as a
// write to a system log that has stopped accepting them.
// `--test-rig-hang-after-secs` reproduces that case on a timer, and carries
// the same pairing requirement as `--test-rig-force-recovery-after-secs`,
// tested the same way here.
// See docs/design/failure-handling.md#test-rig-probes.

/// `--test-rig-hang-after-secs` without `--test-rig-no-sidecar` is refused on
/// the shape of the command line alone, before the asset is read, like
/// `force_recovery_without_test_rig_no_sidecar_refused_exit_2`.
#[test]
fn test_rig_hang_without_test_rig_no_sidecar_refused_exit_2() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--test-rig-hang-after-secs", "5"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("--test-rig-no-sidecar"),
        "stderr must explain the required pairing: {}",
        r.stderr
    );
}

/// Same missing-value rule as every other flag that takes a value.
#[test]
fn test_rig_hang_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--test-rig-hang-after-secs"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// A non-numeric value refuses with the usage text; it does not parse as 0 and
/// does not panic the process.
#[test]
fn test_rig_hang_flag_non_numeric_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--test-rig-hang-after-secs", "soon"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(REFUSED_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// With `--test-rig-hang-after-secs 0` the hang check fires on the first loop
/// iteration, before that iteration's event dispatch, so the run's own fast
/// exit under `--opt vid=no --opt aid=no` cannot win the race. A parked
/// supervisor thread never exits on its own, so the run ends only at the
/// harness's deadline, and the assertion reads `deadline_killed`.
///
/// This covers the flag parking the thread. This harness runs no systemd, so
/// whether a watchdog then kills the process is a manual run on a Raspberry
/// Pi.
/// See docs/design/development.md#test-only-flags.
#[test]
fn test_rig_hang_flag_parks_the_supervisor_thread_forever() {
    let p = temp_path("hang.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--test-rig-no-sidecar",
            "--fps",
            "30",
            "--test-rig-hang-after-secs",
            "0",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(6),
    );
    assert!(
        r.deadline_killed,
        "an unresponsive supervisor thread must never exit on its own -- exit_code {:?}, \
         stderr: {}",
        r.exit_code, r.stderr
    );
    assert!(
        r.stderr.contains("(test rig only) (hang probe)")
            && r.stderr.contains("--test-rig-hang-after-secs=0 is armed"),
        "startup must warn that the probe is armed: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("parking the supervisor thread forever"),
        "the firing line must show that the probe triggered: {}",
        r.stderr
    );
}
