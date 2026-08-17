//! T3 — failure-path integration tests. Every serious bug in this program
//! lived in a failure path; the happy path was never the problem. Each test
//! asserts EXIT BEHAVIOUR (code + boundedness), not output niceties.
//!
//! These tests spawn the real binary, which links libmpv — this target runs
//! on the Pi (`cargo test`); on the Mac use `cargo test --lib`.
//!
//! DISPLAY SAFETY: a soak may own the display. Every invocation that can
//! reach mpv_create MUST carry `--no-defaults --opt vo=null --opt vid=no
//! --opt aid=no`: the null VO never touches DRM, and deselecting all tracks
//! makes mpv end deterministically (NOTHING_TO_PLAY -> END_FILE) instead of
//! playing forever.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Exit-code contract (see usage()): refused before playback vs runtime failure.
const GATE_EXIT: i32 = 2;
const RUNTIME_EXIT: i32 = 1;

/// Outcome of one run. `exit_code` is None when the process had to be killed
/// at the deadline OR died by signal. For the exit-code tests both are
/// failures the `Some(..)` assertions catch — but for the live-fire survival
/// test the polarity flips (None is the PASS), so the two None causes must be
/// distinguishable: `deadline_killed` is true only when THIS HARNESS killed
/// the child at the deadline. A child that died by signal on its own
/// (SIGSEGV/SIGABRT) has `exit_code: None` with `deadline_killed: false`,
/// and conflating that with survival would let a crashing recovery pass a
/// survival assertion.
struct Run {
    exit_code: Option<i32>,
    deadline_killed: bool,
    stderr: String,
}

/// Spawn dex-loop, wait at most `deadline`, kill on overrun. THE DEADLINE IS
/// THE ASSERTION: a player that hangs on a failure path is this program's
/// worst outcome — alive, supervisor green, screen black. The shipped
/// END_FILE idle-hang was exactly that.
fn run_with_deadline(args: &[&str], deadline: Duration) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dex-loop"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dex-loop");
    // Drain stderr on a thread so a chatty child can never fill the pipe and
    // block — a blocked child would masquerade as a hang.
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
    p.push(format!("dex-loop-test-{}-{}", std::process::id(), name));
    p
}

/// Minimal Annex-B HEVC scaffold: VPS, SPS, PPS, then an IDR_N_LP slice --
/// the exact NAL payloads of a real single-frame x265 encode (16x16 black,
/// closed GOP: `ffmpeg -f lavfi -i color=c=black:s=16x16:d=1:r=1 -frames:v 1
/// -c:v libx265 -x265-params keyint=1 -f hevc`), not synthetic filler.
///
/// This MUST be real, parseable HEVC, not arbitrary filler bytes: `loop://`
/// never returns EOF -- that is the entire point of the player -- and
/// libavformat's probe (`avformat_find_stream_info`) only gives up early
/// when it hits EOF. Verified on-device: fed a garbage VPS/SPS through the
/// endless non-EOF stream, the probe can extract nothing AND never sees
/// end-of-stream, so it keeps demanding more data forever -- CPU pinned at
/// 100%, RSS climbing unbounded, silent (no stderr) until the test deadline
/// kills it. A real SPS lets `avformat_find_stream_info` resolve
/// width/height/profile from the header in one pass, independent of the
/// stream's (infinite) length, so mpv reaches "no video or audio streams
/// selected" -> END_FILE in well under a second.
///
/// Structurally what the F4 NAL gate requires; not meant to actually
/// *decode* -- video is deselected (`--opt vid=no`) before decode is ever
/// attempted, so only stream *identification* needs to succeed.
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

/// Write `<asset>.json` binding `bytes` at `fps` — the deploy-path fixture.
/// Inert before task 6 (the player ignores it); binding afterwards.
fn write_sidecar(asset: &Path, bytes: &[u8], fps: &str) {
    let sha = dex_loop::sha256::sha256_hex(bytes);
    std::fs::write(
        format!("{}.json", asset.display()),
        format!(r#"{{"fps":"{fps}","sha256":"{sha}"}}"#),
    )
    .unwrap();
}

/// F6 — write a minimal, always-satisfiable exhibit config at a unique temp
/// path and return it. `display_mode: "auto"` skips the sysfs mode
/// pre-flight (no WxH to check), and `kms_force: "none"` is paired with
/// `write_no_video_cmdline` below so the cmdline gate passes deterministically
/// regardless of what the REAL host's `/proc/cmdline` happens to contain —
/// deploy-path tests below pass BOTH this and `--proc-cmdline
/// <write_no_video_cmdline path>` so the F6 gates are satisfied without
/// depending on the test host's kernel command line, and the test's own gate
/// (F3/F4/opt) still runs exactly as it did before F6 existed.
fn write_exhibit_config(name: &str) -> PathBuf {
    let p = temp_path(name);
    std::fs::write(&p, r#"{"display_mode":"auto","kms_force":"none"}"#).unwrap();
    p
}

/// F6 — a synthetic `/proc/cmdline` with no `video=` token at all, so the
/// cmdline gate's "kms_force=none, expect no token" branch always matches,
/// independent of the real host's actual kernel command line (a CI container
/// has none either way, but the real bench Pi, once F6 is deployed there,
/// legitimately does).
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
            "/nonexistent/dex-loop-test.265",
            "--fps",
            "30",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("/nonexistent/dex-loop-test.265"),
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    // Pin the INTENDED gate (main.rs's empty-payload guard), not just any
    // refusal: without this, deleting that guard would leave the test green
    // via the (also exit-2) missing-sidecar path instead.
    assert!(r.stderr.contains("is empty"), "stderr: {}", r.stderr);
}

// REMOVED 2026-08-17: `no_args_exits_2_with_usage`, which asserted that a bare
// `dex-loop` prints usage and exits 2.
//
// It was correct until F6 moved the asset into the exhibit config. Now
// `ExecStart=/usr/bin/dex-loop` passes exactly zero arguments, so "no args" is
// the SHIPPED invocation rather than an operator error -- and this test caught
// that regression on the Pi within minutes, which it could only ever have done
// there: the Mac cannot link these targets at all.
//
// Not merely inverted to "no args must NOT print usage", because on a card
// whose config and asset are both complete, a bare run would START PLAYBACK
// and take DRM master inside a test -- a visible glitch on a device that may
// be mid-exhibition, and a failing assertion anyway (the harness deadline, not
// an exit code). A test must not be able to interrupt a show.
//
// The property it protected -- a missing positional is not an argv error --
// is covered host-independently by
// `no_positional_asset_is_accepted_and_reaches_the_config` below, which
// supplies its own --exhibit-config and lands on a gate rather than on
// playback.

/// A `--fps`/`--mode` with no following value used to evaporate silently
/// (`args.get(i)` -> `None` -> the flag is just dropped) instead of refusing:
/// an edited systemd unit or a line-continuation typo would start the player
/// on the connector-preferred mode, or with no fps cross-check, with zero
/// error. `--opt` already fell into `usage()` on a missing value -- these two
/// flags must too.
#[test]
fn fps_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(&["/nonexistent/x.265", "--fps"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

#[test]
fn mode_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(&["/nonexistent/x.265", "--mode"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// THE regression for shipped bug #1: mpv_create enables idle mode, so a
/// playback failure emits END_FILE and then idles FOREVER unless the event
/// loop treats END_FILE as fatal. Force a deterministic, display-free
/// playback failure (vid=no + aid=no deselect every track -> mpv ends with
/// "nothing to play" -> END_FILE) and assert the process EXITS, code 1,
/// within the deadline. Before the END_FILE fix this exact scenario sat in
/// idle indefinitely with the supervisor reading green.
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
        "player HUNG on a playback failure (killed at deadline); stderr: {}",
        r.stderr
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    // Exit 1 is also returned by four OTHER failure sites (mpv_create,
    // set_opt, mpv_initialize, loadfile). Without this, a regression that
    // makes one of those fail instead -- e.g. `--opt` plumbing silently
    // broken so `vo=null` never reaches mpv -- would produce an instant exit
    // 1 and this test would stay green while no longer exercising the
    // END_FILE branch at all. Pin the branch, not just the exit code.
    assert!(
        r.stderr.contains("playback ended"),
        "exited 1 but not via the END_FILE branch this test exists to pin: {}",
        r.stderr
    );
}

/// Post-F4: garbage never reaches mpv — the NAL gate refuses it at startup,
/// fast, with a message naming the actual problem. (The pre-F4 version of
/// this test allowed exit 1 via mpv's demux-probe failure; the event-loop
/// hang class is covered by playback_failure_exits_nonzero_never_hangs.)
#[test]
fn garbage_bytes_refused_at_the_gate_exit_2() {
    let p = temp_path("garbage.265");
    // 64 KiB of bytes in 0x02..=0x7E: no 0x00/0x01 -> no start code anywhere.
    let bytes: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761) >> 24) as u8 % 0x7d) + 0x02)
        .collect();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30"); // hash MATCHES: proves the gate, not F3, refuses
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("start code"), "stderr: {}", r.stderr);
}

/// Wrong-but-intact: a CRA-led (open GOP) asset with a CORRECT sidecar hash.
/// F3 passes — the bytes are exactly what was ingested — and F4 must still
/// refuse, proving the hash alone is insufficient.
#[test]
fn open_gop_asset_refused_at_the_gate_exit_2() {
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("CRA"), "stderr: {}", r.stderr);
}

// ---- F3: sidecar binding (task 6) ---------------------------------------

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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains(&format!("{}.json", p.display())),
        "stderr must name the sidecar path: {}",
        r.stderr
    );
}

/// The truncated-copy case F4 can NEVER catch: leading NALs intact, tail
/// missing. Only the hash sees it. The sidecar binds the FULL bytes; the
/// file on disk is truncated.
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("25") && r.stderr.contains("30"),
        "stderr must name both rates: {}",
        r.stderr
    );
}

/// Exit 1 — the mpv path — proves the gates PASSED with an agreeing --fps.
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

/// A rejected mpv option is a deterministic, operator-fixable bad invocation
/// -- the same asset + flags fail identically on every restart -- so per the
/// exit-code contract it must be 2 ("fix and redeploy"), not 1 ("the
/// supervisor restarts"): reading it as transient sends deploy-night triage
/// looking in the wrong place.
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
}

/// SUSPECTED-then-confirmed (adversarial review, 2026-08-15): a rational fps
/// string reaches mpv's `container-fps-override` and is accepted end to end.
/// Verified live on the bench Pi at review time via manual invocation; pinned
/// here so a future mpv, or a future edit to how fps is plumbed, cannot
/// silently regress every NTSC-rate asset into an exit-1 boot loop.
#[test]
fn rational_fps_reaches_playback() {
    let p = temp_path("ntsc.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // bench path: no sidecar needed
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
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

/// Exit 1, not 2: the two-flag bench escape hatch bypasses the sidecar gate
/// and reaches playback with no sidecar on disk.
#[test]
fn bench_escape_hatch_bypasses_sidecar() {
    let p = temp_path("bench.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // deliberately no sidecar
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
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

#[test]
fn bench_flag_without_fps_refused_exit_2() {
    let p = temp_path("benchnofps.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
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
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
}

// ---- F7: build identity + heartbeat (task 8) -----------------------------

#[test]
fn startup_identifies_version_and_build() {
    // Even a refused start must identify its build — a field journal that
    // begins with an unidentifiable process is undebuggable weeks later.
    let r = run_with_deadline(&["/nonexistent/x.265"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT));
    assert!(
        r.stderr
            .contains(&format!("dex-loop {}", env!("CARGO_PKG_VERSION"))),
        "stderr: {}",
        r.stderr
    );
}

// ---- F1: tier-0 health check does not disrupt normal operation ----------

/// F1 registers `mpv_observe_property("time-pos", ...)` unconditionally
/// whenever `mpv_initialize` succeeds -- i.e. on every test above that
/// reaches playback. This pins that registration succeeding SILENTLY (no
/// "mpv_observe_property" warning in stderr) as its own assertion, so a
/// future FFI slip (wrong arg order, wrong format constant, wrong function
/// signature) that makes registration fail -- but not crash -- gets a
/// dedicated regression test instead of only ever showing up as a
/// silently-disabled safety net nobody notices.
///
/// What this does NOT exercise: an actual stall + in-place recovery +
/// escalation. Doing that safely would need real decode with a selected
/// video track and a bounded-but-nonzero wait for two ~10s health-check
/// ticks to elapse -- and this suite's mandatory `--opt vid=no` (see the
/// module doc above) exists specifically to forbid letting any CLI test
/// reach real decode, because the endless-stream design means such a test
/// could never end on its own except by being killed at a deadline. The
/// escalation POLICY itself (attempts, thresholds, when it gives up, the
/// position-baseline reset across a recovery) is pure logic and is
/// exhaustively tested in src/health.rs with none of that risk; this test
/// is the narrow slice of the mpv-facing half that CAN be exercised here
/// without touching decode or the display. The full mpv-facing behaviour
/// (real time-pos progressing, a real health-check tick, a real recovery)
/// is verified manually on the Pi against the actual display and asset --
/// see the crate's PLAN.md F1 entry and this task's session notes.
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
        "F1's time-pos subscription failed to register: {}",
        r.stderr
    );
}

#[test]
fn heartbeat_zero_is_emitted_at_startup() {
    // The 10-minute cadence is untestable in a test budget; heartbeat #0
    // right after loadfile proves temperature reading and line formatting
    // on every boot — and therefore here. Since F9 it does NOT prove the
    // mpv property subscriptions (frame-drops/vo-delayed/pos read "n/a" at
    // heartbeat #0 by design, since nothing has decoded yet); that needs a
    // longer-running on-device check, not this test.
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
        r.stderr.contains("heartbeat wraps="),
        "stderr: {}",
        r.stderr
    );
}

// ---- T7: --force-recovery-after-secs, the live-fire bench probe ---------
//
// C1 (PLAN.md's F1 addendum) shipped and reached the bench without ever
// having been exercised against a live mpv: every in-place recovery killed
// the process on its own first step, and nothing in the suite would have
// caught it. T7 closes that gap with a bench-only flag that forces the
// SAME `AttemptRecovery` decision an organic stall would, on a timer,
// during otherwise-healthy playback -- see dex_loop::health's
// `HealthMonitor::force_recovery` / `ForceRecoveryTrigger` for the pure
// logic and main.rs's `act_on_health_action` for why a forced probe drives
// the identical mpv-facing mechanics a real stall would.
//
// The tests below stay INSIDE this file's mandatory `--opt vid=no --opt
// aid=no` rule (see the module doc at the top of this file): they prove the
// CLI plumbing -- the flag parses, the "impossible to enable accidentally
// in a deployment" gate refuses it without --bench-no-sidecar, and the loud
// arming warning prints -- without ever letting the forced trigger actually
// fire, since firing needs a health check tick against playback that is
// still alive, and vid=no/aid=no makes mpv reach "nothing to play" and end
// in well under a second (see stub_annexb's doc comment). Actually
// observing the forced recovery succeed against a live mpv needs REAL
// decode, which this file's rule exists to keep out of the automated suite
// -- that scenario is the #[ignore]d test below instead.

/// The gate itself (main.rs, checked on CLI shape alone, before the asset
/// is even read): `--force-recovery-after-secs` without `--bench-no-sidecar`
/// is refused, regardless of what -- if anything -- exists on disk at the
/// given path. This is the "impossible to enable accidentally in a
/// deployment" requirement, made concrete: a real deployment's ExecStart
/// never passes --bench-no-sidecar (deploy/dex-loop.service always binds a
/// real sidecar), so this flag can never end up armed against a gallery
/// show, however it got pasted into a command line.
#[test]
fn force_recovery_without_bench_no_sidecar_refused_exit_2() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--force-recovery-after-secs", "5"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("--bench-no-sidecar"),
        "stderr must explain the required pairing: {}",
        r.stderr
    );
}

/// Same missing-value discipline as `--fps`/`--mode`
/// (fps_flag_missing_value_refused_exit_2_with_usage): a flag at the end of
/// argv with no following value must refuse loudly via usage(), not
/// evaporate into "flag absent".
#[test]
fn force_recovery_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--force-recovery-after-secs"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// A non-numeric value must also refuse via usage(), not silently parse as
/// 0 or panic the process.
#[test]
fn force_recovery_flag_non_numeric_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &[
            "/nonexistent/x.265",
            "--force-recovery-after-secs",
            "soon",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// Paired with --bench-no-sidecar (the only way the gate above ever
/// accepts it), the flag is armed: startup must print the loud "BENCH ONLY
/// (T7)... ARMED" warning, and playback must proceed exactly as it does
/// without the flag (vid=no/aid=no's deterministic fast exit via "nothing
/// to play"). `N` is large enough that the forced trigger provably never
/// gets a chance to fire before that fast exit, so this test cannot
/// flake on the race between the two -- it exists to prove the plumbing
/// and the warning, not the live-fire behaviour itself.
#[test]
fn force_recovery_flag_with_bench_no_sidecar_arms_and_reaches_playback() {
    let p = temp_path("forcerecovery.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // bench path: no sidecar needed
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
            "--fps",
            "30",
            "--force-recovery-after-secs",
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
        r.stderr.contains("BENCH ONLY (T7)") && r.stderr.contains("ARMED"),
        "must print the loud arming warning: {}",
        r.stderr
    );
    // The trigger must not have fired in this short a run -- if it had,
    // that would mean it fired against a process already past "nothing to
    // play", which is not the scenario this test is designed to prove.
    assert!(
        !r.stderr.contains("T7 bench probe"),
        "the forced trigger must not have had a chance to fire here: {}",
        r.stderr
    );
}

/// T7 -- LIVE-FIRE test for F1's in-place recovery against a REAL mpv
/// instance: forces a tier-0 recovery a few seconds into otherwise-healthy
/// playback and asserts the process SURVIVES it (recovery absorbed, still
/// running) -- the exact scenario C1 broke. `is_expected_recovery_stop`'s
/// pure-logic tests in main.rs pin the boolean condition that fixes C1;
/// this test is the live-fire check that a REAL mpv event stream actually
/// produces the shape that condition expects.
///
/// Deliberately NOT part of the automated (default) suite: unlike every
/// other test in this file, it does NOT pass `--opt vid=no` -- it needs the
/// real video track selected so time-pos actually advances and a
/// health-check tick can observe "healthy" before the forced trigger fires.
/// `--opt vo=null` keeps it headless (no DRM, no display touched) but does
/// NOT bound the decode: only killing at run_with_deadline's deadline does,
/// same as it would for any endless-stream real-decode run. This is exactly
/// the deviation this file's module doc says the mandatory vid=no/aid=no
/// rule exists to keep out of the automated suite -- hence `#[ignore]`, not
/// a relaxation of that rule for anything else here. `#[ignore]` also keeps
/// this out of a Pi mid-soak's plain `cargo test`, which must not add
/// unrelated CPU load to a thermal measurement.
///
/// **CI now runs this test deliberately**, by exact name, as its own step
/// in `.github/workflows/dex-loop-deb.yml` (after `Test`, before
/// `Build package`) -- the debian:trixie container has no display and no
/// DRM, so it exercises the same headless `vo=null` + software-HEVC-decode
/// path this doc comment describes, with no code path skipped. Manual
/// invocation (e.g. on the Pi, against `dexpi4.local`) still works exactly
/// as before:
///
///   cargo test --test cli force_recovery_survives_against_real_mpv -- --ignored --nocapture
///
/// DO NOT manually run this on dexpi4.local while its thermal soak is
/// active (its tmux sessions "dexeye"/"thermal" own the display and the CPU
/// is being measured -- see this task's hard constraint). Once the soak has
/// concluded, or on any OTHER Pi 4 (or dev machine) with libmpv 0.40+
/// installed and nothing else on the display, the command above is safe.
///
/// Expected stderr, in order:
///   1. "dex-loop 0.1.0 (...)"                                -- normal startup
///   2. "warning: BENCH ONLY (T7): --force-recovery-after-secs=3 is ARMED"
///   3. (a few seconds of nothing -- real decode, no per-frame logging)
///   4. "dex-loop: health check: T7 bench probe: ... -- attempting in-place
///      recovery 1/3 ..."
///   5. "dex-loop: health check: in-place recovery's loadfile replaced the
///      stream; absorbing the expected END_FILE(reason=stop) ..."
///   6. process is STILL RUNNING when this test kills it at its deadline,
///      and never printed a SECOND "attempting in-place recovery 2/" (see
///      the assertion below for why that matters at this deadline).
///
/// If C1 has regressed: step 5 never appears, and the process exits 1 right
/// after step 4 instead (the recovery's own END_FILE(reason=stop) treated
/// as fatal) -- well before the deadline.
///
/// **What this test does (and does not) prove.** It proves the process
/// survives its own recovery and that time-pos resumes advancing afterwards
/// (the recovery-2/-absence check below), under software decode and
/// `vo=null`, with no display. It does NOT prove the picture actually comes
/// back on real hardware: that claim lives entirely in the option set this
/// test never exercises -- `hwdec=drm`, `gpu-hwdec-interop=drmprime-overlay`,
/// the swapped DRM plane assignment -- all skipped here via `--no-defaults`.
/// A recovery whose `loadfile replace` tears down and rebuilds the
/// DRM/hwdec chain incorrectly could leave a black screen with a perfectly
/// alive, CI-green process. Only the on-Pi bench run with a real display
/// (this test's invocation above, minus `--no-defaults`/`vo=null`, against
/// the actual exhibition display) closes that gap -- CI proves survival,
/// only the bench proves the picture.
///
/// **Residual gap even within what CI can see, stated rather than papered
/// over:** the recovery-2/-absence assertion below proves "no SECOND
/// recovery fired organically", which requires the event loop to still be
/// alive and ticking (mpv_wait_event waking on its timeout, HealthMonitor
/// still being ticked) even if it never got a second STALL to react to. A
/// process whose event loop wedged COMPLETELY right after the absorb --
/// mpv_wait_event itself never returning again, no further ticks at all --
/// would produce neither a second "attempting in-place recovery" line NOR
/// an exit, and every assertion here (still alive, attempt 1/, absorption,
/// no attempt 2/) would pass vacuously. Closing that fully would need a
/// positive post-recovery signal (e.g. a bench-only per-tick "healthy" log
/// line while T7 is armed, asserted present at least once after the
/// absorb) -- not implemented here; this comment exists so that gap is
/// recorded rather than silently assumed covered.
#[test]
#[ignore]
fn force_recovery_survives_against_real_mpv() {
    let p = temp_path("liverecovery.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
            "--fps",
            "30",
            "--force-recovery-after-secs",
            "3",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    // Survival means "still running when THIS TEST killed it at the deadline"
    // -- asserted via `deadline_killed`, not via `exit_code == None`, because
    // death by signal (SIGSEGV/SIGABRT) also yields `exit_code: None`. A
    // C1-class regression that crashed via signal AFTER printing the absorb
    // line would pass an exit_code-shaped assertion; it cannot pass this one.
    assert!(
        r.deadline_killed,
        "process must SURVIVE the forced recovery (still running when killed \
         at the deadline); it ended on its own with exit_code {:?} -- an exit \
         means the recovery's own END_FILE(reason=stop) was NOT absorbed, and \
         exit_code None here means death by signal -- either way C1 has \
         regressed. stderr: {}",
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
        "recovery's own END_FILE was not absorbed -- this is exactly C1: {}",
        r.stderr
    );
    // The three assertions above are satisfiable by a process that
    // "survives" only because its event loop wedged solid right after
    // absorbing the stop -- alive, but not actually playing. A genuine
    // recovery lets time-pos resume advancing, which feeds the health
    // monitor a fresh "healthy" sample and means NO second recovery gets
    // triggered organically. At a 30s deadline (trigger at 3s, health-check
    // ticks every ~10s) a wedged-but-alive process would reach a second
    // organic attempt at roughly trigger+20s =~ 23s -- comfortably inside
    // this deadline -- so this string's ABSENCE is a real "playback
    // actually resumed" proxy, not decoration. (It would be vacuously true
    // at the old 15s deadline, which is why the deadline was raised.)
    assert!(
        !r.stderr.contains("attempting in-place recovery 2/"),
        "a second recovery fired organically after the forced one -- time-pos \
         likely never resumed advancing post-recovery (event loop wedged \
         rather than truly recovered): {}",
        r.stderr
    );
}

// ---- F6: the exhibit display config --------------------------------------
//
// The pure decision surface (grammar, resolve_display, the cmdline
// comparator, reconcile_cmdline, the sysfs pre-flight parser) is exhaustively
// tested in src/exhibit.rs on the Mac -- these tests exist only to pin how
// main.rs WIRES that logic in: gate ORDER (display gates fire before the
// asset is even read), the new flags parse, and the CLI-level refusal
// messages. All of them stay inside this file's mandatory `vo=null --opt
// vid=no --opt aid=no` rule.

/// An `--exhibit-config` naming a file that is not there, and no
/// `--bench-no-sidecar`: refused before even the asset path is looked at, and
/// refused NAMING THE PATH THE OPERATOR GAVE.
///
/// The naming half is the point. Until the dual-format work this borrowed
/// `resolve_display`'s "no exhibit config — create /etc/dex/exhibit.json"
/// message, which is wrong advice for someone who just pointed the flag
/// somewhere else: they would create a file the run they are debugging does
/// not read. Same class as the EACCES fix on the read path.
#[test]
fn missing_exhibit_config_refused_exit_2_before_the_asset_is_read() {
    let improbable = temp_path("f6-default-exhibit-config-must-not-exist.json");
    let _ = std::fs::remove_file(&improbable); // never written; just proving absence
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-missing-config.265",
            "--exhibit-config",
            improbable.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("f6-default-exhibit-config-must-not-exist.json"),
        "the refusal must name the path that was actually given: {}",
        r.stderr
    );
    // The load-bearing negative: the asset gate must NOT have run yet.
    assert!(
        !r.stderr.contains("f6-missing-config.265"),
        "the exhibit gate should refuse BEFORE the asset path is even looked \
         at, but the asset-missing message appeared too: {}",
        r.stderr
    );
}

// The complementary case -- NO --exhibit-config and no installed default, so
// resolve_display's "no exhibit config, create the shipped one" message fires
// -- is deliberately NOT tested here. It would depend on the HOST lacking
// /etc/dex, which is true on the Mac and false on the Pi (where the .deb
// installs exactly that file), so it would assert one thing in development and
// silently something else on the device -- the vacuous-check class this file
// has been bitten by before. It is covered where it is host-independent:
// `load_returns_none_when_no_default_exists` and `resolve_display`'s own unit
// tests in src/exhibit.rs.

/// A YAML exhibit config drives the real binary end to end — the same gate
/// order, from a `.yaml` file. Pins that the format dispatch is wired into
/// main.rs and not merely unit-tested in the library.
#[test]
fn yaml_exhibit_config_binds_the_display_like_json_does() {
    let cfg = temp_path("f6-yaml-exhibit.yaml");
    std::fs::write(
        &cfg,
        "# a venue would really write this\ndisplay_mode: 7680x4320@60\nkms_force: none\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("f6-yaml-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-yaml.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    // Reaching the sysfs pre-flight (which refuses this implausible 8K mode)
    // proves the YAML parsed, validated, and bound the display: a config that
    // had failed earlier could not produce THIS message.
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("7680x4320"), "stderr: {}", r.stderr);
}

/// **The shipped deployment invocation shape**: no positional asset path at
/// all, everything from the exhibit config. `ExecStart=/usr/bin/dex-loop`
/// passes exactly this, so if a no-argument run were rejected on argv shape,
/// the packaged unit would exit 2 in a permanent restart loop on the device
/// while every other test here — all of which pass arguments — stayed green.
///
/// Uses `--exhibit-config` (never the real default) so the test does not
/// depend on the host having, or lacking, `/etc/dex`; and an implausible mode,
/// so it lands on the sysfs pre-flight rather than starting playback.
#[test]
fn no_positional_asset_is_accepted_and_reaches_the_config() {
    let cfg = temp_path("f6-noposition-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/f6-fromconfig.265\ndisplay_mode: 7680x4320@60\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("f6-noposition-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    // Reached a real gate, NOT the usage text.
    assert!(
        !r.stderr.contains("usage:"),
        "a no-argument run must not be refused on argv shape -- that is how the \
         packaged unit invokes the player: {}",
        r.stderr
    );
    assert!(r.stderr.contains("7680x4320"), "stderr: {}", r.stderr);
}

/// The asset actually comes FROM the config: with a valid display and a config
/// naming a nonexistent asset, the run gets as far as failing to read that
/// exact path — which only happens if `resolve_asset` took it from the file.
#[test]
fn the_exhibit_config_names_which_asset_plays() {
    let cfg = temp_path("f6-assetfromconfig-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/f6-named-by-config.265\ndisplay_mode: auto\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("f6-assetfromconfig-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("f6-named-by-config.265"),
        "the asset named by the config must be the one the player tried to read: {}",
        r.stderr
    );
}

/// A positional path that CONTRADICTS the config is refused naming both — the
/// asset analogue of `mode_contradicting_exhibit_config_refused_naming_both`.
#[test]
fn positional_asset_contradicting_the_config_refused_naming_both() {
    let cfg = temp_path("f6-assetconflict-exhibit.yaml");
    std::fs::write(
        &cfg,
        "asset: /nonexistent/f6-config-asset.265\ndisplay_mode: auto\n",
    )
    .unwrap();
    let cl = write_no_video_cmdline("f6-assetconflict-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-cli-asset.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("f6-config-asset.265") && r.stderr.contains("f6-cli-asset.265"),
        "stderr must name both assets: {}",
        r.stderr
    );
}

/// A config with no `asset`, and no path given: refuses rather than falling
/// back to /opt/dex/loop.265. The fail-closed row of `resolve_asset`'s table,
/// driven through the real binary.
#[test]
fn no_asset_named_anywhere_refused_without_guessing_loop_265() {
    let cfg = temp_path("f6-noasset-exhibit.yaml");
    std::fs::write(&cfg, "display_mode: auto\n").unwrap();
    let cl = write_no_video_cmdline("f6-noasset-cmdline");
    let r = run_with_deadline(
        &[
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("no asset"), "stderr: {}", r.stderr);
    // The load-bearing negative: it must not have quietly tried the old
    // hardcoded path.
    assert!(
        !r.stderr.contains("cannot read /opt/dex/loop.265"),
        "a missing asset must never fall back to the pre-F6 hardcoded path: {}",
        r.stderr
    );
}

/// The dispatch, at the CLI level: YAML syntax inside a `.json` file is
/// refused rather than quietly accepted by a permissive parser.
#[test]
fn yaml_contents_in_a_json_named_config_refused() {
    let cfg = temp_path("f6-yaml-in-json.json");
    std::fs::write(&cfg, "display_mode: 3840x2160@30\n").unwrap();
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-yamlinjson.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("JSON"), "stderr: {}", r.stderr);
}

/// An unparseable exhibit config is refused with the specific parse error —
/// distinct from "missing", per main.rs's read/parse split (see the comment
/// at the exhibit_config read site).
#[test]
fn malformed_exhibit_config_refused_exit_2_naming_the_parse_error() {
    let cfg = temp_path("f6-malformed-exhibit.json");
    std::fs::write(&cfg, r#"{"display_mode":"auto","kms_forse":"none"}"#).unwrap(); // typo
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-malformed.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("kms_forse"), "stderr: {}", r.stderr);
}

/// `--bench-no-sidecar` bypasses BOTH the exhibit config AND the cmdline
/// gate: no `--exhibit-config` is supplied, no `--proc-cmdline` is supplied,
/// and the run still reaches playback (exit 1, not 2) because bench mode
/// consults neither.
#[test]
fn bench_flag_bypasses_the_exhibit_config_and_cmdline_gate_too() {
    let p = temp_path("f6-bench.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
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

/// A `--mode` that contradicts the exhibit config's `display_mode` is
/// refused, naming both — the F6 analogue of
/// `fps_contradicting_sidecar_refused_exit_2_naming_both`.
#[test]
fn mode_contradicting_exhibit_config_refused_naming_both() {
    let cfg = temp_path("f6-modeconflict-exhibit.json");
    std::fs::write(&cfg, r#"{"display_mode":"3840x2160@30","kms_force":"none"}"#).unwrap();
    let cl = write_no_video_cmdline("f6-modeconflict-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-modeconflict.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
            "--mode",
            "2560x1440@60",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("3840x2160@30") && r.stderr.contains("2560x1440@60"),
        "stderr must name both modes: {}",
        r.stderr
    );
}

/// The cmdline gate: exhibit config says `kms_force=none`, but the (fixture)
/// kernel cmdline carries a `video=HDMI-A-1:...` token anyway — the exact
/// 2026-08-15 incident class (a force removed as "stale" while still in use,
/// or here, the mirror case: a config edited to "none" while the boot config
/// was never reconciled). Must refuse naming the fix, before the asset is
/// even read.
#[test]
fn cmdline_mismatch_refused_naming_dex_exhibit_apply() {
    let cfg = write_exhibit_config("f6-cmdlinemismatch-exhibit.json"); // kms_force: none
    let cl = temp_path("f6-cmdlinemismatch-cmdline");
    std::fs::write(&cl, "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait\n").unwrap();
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-cmdlinemismatch.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("dex-exhibit-apply") && r.stderr.contains("3840x2160@30"),
        "stderr: {}",
        r.stderr
    );
}

/// The sysfs mode pre-flight: a non-"auto" display_mode that no real
/// connector could plausibly offer (8K60 — no HDMI-A-1 sink on a CI runner OR
/// this project's actual bench displays advertises this) is refused, either
/// because the connector cannot be found at all (a CI container with no DRM)
/// or because it is found but does not list the mode — the two-layer
/// "cannot find" vs "not among the modes" split from the F6 design's §2.4.
/// Either message names the requested resolution, which is what this test
/// pins portably across both hosts.
#[test]
fn implausible_mode_refused_by_the_sysfs_preflight() {
    let cfg = temp_path("f6-implausible-exhibit.json");
    std::fs::write(
        &cfg,
        r#"{"display_mode":"7680x4320@60","kms_force":"none"}"#,
    )
    .unwrap();
    let cl = write_no_video_cmdline("f6-implausible-cmdline");
    let r = run_with_deadline(
        &[
            "/nonexistent/f6-implausible.265",
            "--exhibit-config",
            cfg.to_str().unwrap(),
            "--proc-cmdline",
            cl.to_str().unwrap(),
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("7680x4320"),
        "stderr must name the implausible resolution: {}",
        r.stderr
    );
    // And it must not be the asset-missing message -- the display gates run
    // first, exactly like the "no exhibit config" case above.
    assert!(!r.stderr.contains("f6-implausible.265"), "stderr: {}", r.stderr);
}

// ---- F10: --bench-wedge-after-secs, the systemd-watchdog live-fire probe -
//
// F9 proved a wedged mpv core produces SILENCE on this program's event
// thread, and F1 acts on that silence in-process. Neither covers the event
// thread hanging in code that is NOT an mpv call at all (e.g. `eprintln!`
// against a wedged journald) -- see dex_loop::watchdog's module doc
// ("Framing"). `--bench-wedge-after-secs` deliberately reproduces that one
// remaining hazard class on a timer, so its plumbing gets the same
// "impossible to enable accidentally in a deployment" gate as T7's
// `--force-recovery-after-secs`, tested the same way here.

/// The gate itself, mirroring
/// `force_recovery_without_bench_no_sidecar_refused_exit_2`:
/// `--bench-wedge-after-secs` without `--bench-no-sidecar` is refused on CLI
/// shape alone, before the asset is even read.
#[test]
fn bench_wedge_without_bench_no_sidecar_refused_exit_2() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--bench-wedge-after-secs", "5"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("--bench-no-sidecar"),
        "stderr must explain the required pairing: {}",
        r.stderr
    );
}

/// Same missing-value discipline as every other flag taking a value.
#[test]
fn bench_wedge_flag_missing_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--bench-wedge-after-secs"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// A non-numeric value must also refuse via usage(), not silently parse as
/// 0 or panic the process.
#[test]
fn bench_wedge_flag_non_numeric_value_refused_exit_2_with_usage() {
    let r = run_with_deadline(
        &["/nonexistent/x.265", "--bench-wedge-after-secs", "soon"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// The mechanism itself: with `--bench-wedge-after-secs 0`, the wedge check
/// fires on the very first loop iteration, unconditionally, BEFORE that same
/// iteration's event-id dispatch can act on whatever `mpv_wait_event`
/// happened to return (see the firing site's comment in main.rs for why
/// that ordering matters -- with `--opt vid=no --opt aid=no`, mpv reaches
/// "nothing to play" and would otherwise race this probe to an ordinary
/// `exit(1)`). A process that has genuinely wedged never exits on its own,
/// so the ONLY way this test ends is the harness's own deadline kill --
/// `deadline_killed` must be true, mirroring the `Run` struct's own doc
/// comment on why that is the correct assertion (an exit_code of `None`
/// alone cannot distinguish "wedged, harness killed it" from "died by
/// signal on its own").
///
/// This proves the MECHANISM -- that the flag genuinely, permanently parks
/// the event thread -- not that a systemd watchdog then kills it: this
/// harness has no systemd to observe. That half is proved on the Pi; see
/// PLAN.md's F10 entry and README.md for the on-device procedure
/// (journalctl showing `Watchdog timeout`, a SIGABRT, and a supervisor
/// restart).
#[test]
fn bench_wedge_flag_actually_hangs_the_event_thread_forever() {
    let p = temp_path("wedge.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
            "--fps",
            "30",
            "--bench-wedge-after-secs",
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
        "a genuinely wedged event thread must never exit on its own -- exit_code {:?}, \
         stderr: {}",
        r.exit_code, r.stderr
    );
    assert!(
        r.stderr.contains("BENCH ONLY (F10 wedge probe)") && r.stderr.contains("ARMED"),
        "must print the loud arming warning: {}",
        r.stderr
    );
    assert!(
        r.stderr
            .contains("deliberately parking the event thread forever"),
        "must print the firing line proving the probe actually triggered: {}",
        r.stderr
    );
}
