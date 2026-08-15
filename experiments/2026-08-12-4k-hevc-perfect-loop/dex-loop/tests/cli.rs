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
/// at the deadline OR died by signal — both are failures the assertions catch.
struct Run {
    exit_code: Option<i32>,
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
    let exit_code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code(),
            None if start.elapsed() > deadline => {
                child.kill().ok();
                child.wait().ok();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stderr = reader.join().expect("stderr reader");
    Run { exit_code, stderr }
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

#[test]
fn missing_file_exits_2_and_names_the_path() {
    let r = run_with_deadline(
        &["/nonexistent/dex-loop-test.265", "--fps", "30"],
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
    let r = run_with_deadline(
        &[p.to_str().unwrap(), "--fps", "30"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
}

#[test]
fn no_args_exits_2_with_usage() {
    let r = run_with_deadline(&[], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT));
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
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
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
    assert!(
        r.exit_code.is_some(),
        "player HUNG on a playback failure (killed at deadline); stderr: {}",
        r.stderr
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
}

/// Undecodable garbage must produce a BOUNDED, nonzero exit — never a hang.
/// Today the garbage reaches mpv and fails its (bounded) demux probe
/// (LOADING_FAILED -> END_FILE -> exit 1). Once the F4 NAL gate lands, the
/// same input is refused before mpv starts (exit 2); task 7 tightens this
/// assertion to exactly that.
#[test]
fn garbage_bytes_exit_nonzero_within_deadline() {
    let p = temp_path("garbage.265");
    // 64 KiB of bytes in 0x02..=0x7E: no 0x00/0x01 (no Annex-B start code
    // anywhere) and no 0xFF (no MP3/ADTS sync word a demuxer could latch onto).
    let bytes: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761) >> 24) as u8 % 0x7d) + 0x02)
        .collect();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
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
    assert!(
        r.exit_code.is_some(),
        "player HUNG on garbage input; stderr: {}",
        r.stderr
    );
    assert_ne!(r.exit_code, Some(0), "stderr: {}", r.stderr);
}
