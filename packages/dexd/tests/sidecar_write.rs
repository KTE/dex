//! End-to-end tests for `dex-sidecar write`, run against real HEVC streams.
//!
//! The streams are encoded here with ffmpeg rather than checked in, so the
//! suite carries no fixtures and every case gets a stream of the shape it is
//! about. Two shapes cover the cases: one whose encoder wrote frame timing
//! into the stream, and one whose encoder left it out, where no frame rate can
//! be read back and `--fps` has to be given.
//!
//! The tests that need a stream fail when ffmpeg and ffprobe are not both on
//! the search path, with a message naming what to install;
//! `DEXD_ALLOW_MEDIA_SKIP=1` turns that failure into a skip on a machine that
//! cannot encode.
//! See docs/design/development.md#test-layers.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Refused before anything was written, or a malformed command line.
/// See docs/design/sidecar.md#dex-sidecar for the exit codes.
const EXIT_REFUSED: i32 = 2;
/// The stream could not be read, or a sidecar failed verification.
const EXIT_FAILED: i32 = 1;

/// Reports whether ffmpeg and ffprobe can both be run.
fn media_tools_present() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Fails the calling test when ffmpeg or ffprobe is missing, unless
/// `DEXD_ALLOW_MEDIA_SKIP` is set. A skipped test still reports a pass, so
/// skipping is opt-in. See docs/design/development.md#test-layers.
macro_rules! needs_ffmpeg {
    () => {
        if !media_tools_present() {
            if std::env::var_os("DEXD_ALLOW_MEDIA_SKIP").is_some() {
                eprintln!(
                    "skipped {}: ffmpeg and ffprobe are not both on the search \
                     path, so no test stream can be made (DEXD_ALLOW_MEDIA_SKIP \
                     is set)",
                    module_path!()
                );
                return;
            }
            panic!(
                "ffmpeg and ffprobe are not both on the search path, so no \
                 test stream can be made. Install them (Debian: apt install \
                 ffmpeg; macOS: brew install ffmpeg), or set \
                 DEXD_ALLOW_MEDIA_SKIP=1 to skip these tests."
            );
        }
    };
}

/// A scratch directory of this test's own, under the system temp directory.
/// It stays after the run so a failure can still be inspected.
fn work_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("dexd-sidecar-write-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&p).expect("create work dir");
    p
}

/// A small, real raw HEVC stream at 30 fps. x265 writes the frame timing into
/// the stream by default, so ffprobe can read the rate back out of this one.
fn stream_with_timing(path: &Path) {
    encode(path, &[]);
}

/// The same stream with the frame timing left out, as many other encoders
/// produce it. ffprobe then reports its internal timebase in place of a frame
/// rate. See docs/design/sidecar.md#frame-rate-at-ingest.
fn stream_without_timing(path: &Path) {
    encode(path, &["-x265-params", "vui-timing-info=0"]);
}

fn encode(path: &Path, extra: &[&str]) {
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x64:rate=30:duration=1",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(extra)
        .args(["-f", "hevc", "-y"])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed to make {}", path.display());
    let len = std::fs::metadata(path).expect("stat stream").len();
    assert!(len > 0, "ffmpeg made an empty stream at {}", path.display());
}

/// Runs dex-sidecar and returns its exit status, stdout and stderr.
fn dex_sidecar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dex-sidecar"))
        .args(args)
        .output()
        .expect("run dex-sidecar")
}

fn exit_code(out: &Output) -> i32 {
    out.status.code().expect("dex-sidecar exited by signal")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Changes one byte in the middle of a file, leaving the stream's leading
/// parameter sets intact.
fn flip_a_byte(path: &Path) {
    let mut bytes = std::fs::read(path).expect("read stream");
    let middle = bytes.len() / 2;
    bytes[middle] = bytes[middle].wrapping_add(1);
    std::fs::write(path, &bytes).expect("write stream");
}

// ---- write, then check ---------------------------------------------------

#[test]
fn a_written_sidecar_passes_check() {
    needs_ffmpeg!();
    let dir = work_dir("roundtrip");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();

    let written = dex_sidecar(&["write", stream]);
    assert_eq!(exit_code(&written), 0, "stderr: {}", stderr(&written));

    // No --fps was given, so the rate came out of the stream itself.
    assert!(
        stdout(&written).contains(r#""fps":"30""#),
        "stdout: {}",
        stdout(&written)
    );
    let sidecar = format!("{stream}.json");
    assert!(Path::new(&sidecar).exists(), "no sidecar at {sidecar}");
    // The dimensions ffprobe reported are written to the sidecar as well.
    let text = std::fs::read_to_string(&sidecar).unwrap();
    assert!(text.contains(r#""width":64"#), "sidecar: {text}");
    assert!(text.contains(r#""height":64"#), "sidecar: {text}");

    let checked = dex_sidecar(&["check", &sidecar, stream]);
    assert_eq!(exit_code(&checked), 0, "stderr: {}", stderr(&checked));
    assert!(stdout(&checked).starts_with("OK "), "{}", stdout(&checked));
}

/// `check` reports a failure once the stream stops matching the sidecar it
/// was written from. See docs/design/sidecar.md#checksum-verification.
#[test]
fn check_fails_once_the_stream_changes() {
    needs_ffmpeg!();
    let dir = work_dir("corrupt");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream_str = stream.to_str().unwrap().to_string();
    let sidecar = format!("{stream_str}.json");

    assert_eq!(exit_code(&dex_sidecar(&["write", &stream_str])), 0);
    flip_a_byte(&stream);

    let checked = dex_sidecar(&["check", &sidecar, &stream_str]);
    assert_eq!(
        exit_code(&checked),
        EXIT_FAILED,
        "stderr: {}",
        stderr(&checked)
    );
    assert!(
        stderr(&checked).to_lowercase().contains("sha256"),
        "the failure must say the hash is what did not match: {}",
        stderr(&checked)
    );
}

// ---- the refusals --------------------------------------------------------

#[test]
fn an_existing_sidecar_is_not_replaced_without_force() {
    needs_ffmpeg!();
    let dir = work_dir("overwrite");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();

    assert_eq!(
        exit_code(&dex_sidecar(&["write", stream, "--fps", "30"])),
        0
    );

    let again = dex_sidecar(&["write", stream, "--fps", "30"]);
    assert_eq!(
        exit_code(&again),
        EXIT_REFUSED,
        "stderr: {}",
        stderr(&again)
    );
    assert!(
        stderr(&again).contains("--force"),
        "the refusal must say how to go ahead anyway: {}",
        stderr(&again)
    );

    let forced = dex_sidecar(&["write", stream, "--fps", "30", "--force"]);
    assert_eq!(exit_code(&forced), 0, "stderr: {}", stderr(&forced));
}

/// A mistyped `--fps` would bind a rate that keeps verifying while the video
/// plays at the wrong speed, so a rate far from the stream's is refused.
/// See docs/design/sidecar.md#frame-rate-at-ingest.
#[test]
fn an_fps_that_contradicts_the_stream_is_refused() {
    needs_ffmpeg!();
    let dir = work_dir("wrongfps");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();
    let out = dir.join("wrongfps.json");
    let out_str = out.to_str().unwrap();

    // 3 where 30 was meant.
    let refused = dex_sidecar(&["write", stream, "--fps", "3", "--out", out_str]);
    assert_eq!(
        exit_code(&refused),
        EXIT_REFUSED,
        "stderr: {}",
        stderr(&refused)
    );
    assert!(
        !out.exists(),
        "a refused write must not leave a sidecar behind"
    );
    assert!(
        stderr(&refused).contains("--force"),
        "the refusal must say how to go ahead anyway: {}",
        stderr(&refused)
    );

    let forced = dex_sidecar(&["write", stream, "--fps", "3", "--out", out_str, "--force"]);
    assert_eq!(exit_code(&forced), 0, "stderr: {}", stderr(&forced));
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(
        text.contains(r#""fps":"3""#),
        "--force must bind the rate given on the command line: {text}"
    );
}

/// A rate within the tolerance of the detected one is accepted without
/// `--force`, whichever way it is spelled.
#[test]
fn an_fps_that_agrees_with_the_stream_needs_no_force() {
    needs_ffmpeg!();
    let dir = work_dir("agreeingfps");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();

    let written = dex_sidecar(&["write", stream, "--fps", "30/1"]);
    assert_eq!(exit_code(&written), 0, "stderr: {}", stderr(&written));
    assert!(
        stdout(&written).contains(r#""fps":"30/1""#),
        "stdout: {}",
        stdout(&written)
    );
}

/// With no timing in the stream there is no rate to read, so `write` refuses
/// and asks for `--fps`.
#[test]
fn fps_is_required_when_the_stream_carries_no_timing() {
    needs_ffmpeg!();
    let dir = work_dir("notiming");
    let stream = dir.join("artwork.265");
    stream_without_timing(&stream);
    let stream = stream.to_str().unwrap();

    let refused = dex_sidecar(&["write", stream]);
    assert_eq!(
        exit_code(&refused),
        EXIT_REFUSED,
        "stderr: {}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("--fps"),
        "the refusal must say what to pass: {}",
        stderr(&refused)
    );
    assert!(
        !Path::new(&format!("{stream}.json")).exists(),
        "a refused write must not leave a sidecar behind"
    );

    // Given the rate, the write succeeds and the sidecar passes `check`.
    let written = dex_sidecar(&["write", stream, "--fps", "30"]);
    assert_eq!(exit_code(&written), 0, "stderr: {}", stderr(&written));
    let sidecar = format!("{stream}.json");
    let checked = dex_sidecar(&["check", &sidecar, stream]);
    assert_eq!(exit_code(&checked), 0, "stderr: {}", stderr(&checked));
}

#[test]
fn a_missing_stream_is_reported_with_its_name() {
    let missing = work_dir("missing").join("nope.265");
    let out = dex_sidecar(&["write", missing.to_str().unwrap(), "--fps", "30"]);
    assert_eq!(exit_code(&out), EXIT_FAILED, "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("nope.265"),
        "the error must name the file: {}",
        stderr(&out)
    );
}

/// The run gets an empty PATH, so ffprobe cannot be found: `--fps` is
/// required, and the sidecar then written leaves out the width and height.
/// The bytes here are never probed, so this test needs no ffmpeg.
#[test]
fn without_ffprobe_the_rate_must_be_given_and_dimensions_are_left_out() {
    let dir = work_dir("noffprobe");
    let stream = dir.join("bytes.265");
    std::fs::write(&stream, b"\x00\x00\x00\x01not really a stream").unwrap();
    let stream = stream.to_str().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_dex-sidecar"))
            .args(args)
            .env("PATH", "")
            .output()
            .expect("run dex-sidecar")
    };

    let refused = run(&["write", stream]);
    assert_eq!(
        exit_code(&refused),
        EXIT_REFUSED,
        "stderr: {}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("--fps"),
        "the refusal must say what to pass: {}",
        stderr(&refused)
    );

    let written = run(&["write", stream, "--fps", "30"]);
    assert_eq!(exit_code(&written), 0, "stderr: {}", stderr(&written));
    let json = stdout(&written);
    assert!(json.contains("\"fps\":\"30\""), "stdout: {json}");
    assert!(
        !json.contains("width") && !json.contains("height"),
        "dimensions cannot be known without ffprobe: {json}"
    );
    let checked = run(&["check", &format!("{stream}.json"), stream]);
    assert_eq!(exit_code(&checked), 0, "stderr: {}", stderr(&checked));
}

#[test]
fn an_empty_stream_is_refused() {
    let dir = work_dir("emptystream");
    let stream = dir.join("empty.265");
    std::fs::write(&stream, b"").unwrap();
    let out = dex_sidecar(&["write", stream.to_str().unwrap(), "--fps", "30"]);
    assert_eq!(exit_code(&out), EXIT_FAILED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("empty"), "stderr: {}", stderr(&out));
}

// ---- command lines that do not make sense --------------------------------

#[test]
fn a_command_line_without_a_subcommand_gets_usage() {
    // Two paths with no subcommand in front of them.
    let out = dex_sidecar(&["artwork.265.json", "artwork.265"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

#[test]
fn no_arguments_at_all_gets_usage() {
    let out = dex_sidecar(&[]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

#[test]
fn an_unknown_subcommand_gets_usage() {
    let out = dex_sidecar(&["generate", "artwork.265"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

/// dex-sidecar refuses a `--fps` with no value after it, so no sidecar is
/// written with a rate nobody chose.
#[test]
fn a_trailing_fps_with_no_value_gets_usage() {
    needs_ffmpeg!();
    let dir = work_dir("danglingfps");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);

    let out = dex_sidecar(&["write", stream.to_str().unwrap(), "--fps"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
    assert!(
        !Path::new(&format!("{}.json", stream.display())).exists(),
        "a refused write must not leave a sidecar behind"
    );
}

#[test]
fn a_trailing_out_with_no_value_gets_usage() {
    let out = dex_sidecar(&["write", "artwork.265", "--out"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

#[test]
fn an_fps_that_is_not_a_frame_rate_is_refused() {
    needs_ffmpeg!();
    let dir = work_dir("badfps");
    let stream = dir.join("artwork.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();

    let out = dex_sidecar(&["write", stream, "--fps", "thirty"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(
        !Path::new(&format!("{stream}.json")).exists(),
        "a refused write must not leave a sidecar behind"
    );
}

#[test]
fn check_with_the_wrong_number_of_paths_gets_usage() {
    let out = dex_sidecar(&["check", "only-one.json"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}
