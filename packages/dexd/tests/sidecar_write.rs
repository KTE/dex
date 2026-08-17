//! End-to-end tests for `dex-sidecar write`, run against real HEVC streams.
//!
//! The streams are made here with ffmpeg rather than checked in, so the suite
//! needs no fixtures and each case gets a stream of exactly the shape it is
//! about. Two shapes are used: one whose encoder wrote frame timing into the
//! stream, and one whose encoder did not — the second is the case where no
//! frame rate can be read back out and the tool has to insist on being told.
//!
//! Without ffmpeg and ffprobe on PATH these tests SKIP, loudly, rather than
//! pass on nothing. CI installs both so they really run there.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Refused before anything was written, or a malformed command line.
const EXIT_REFUSED: i32 = 2;
/// The stream could not be read, or a sidecar failed verification.
const EXIT_FAILED: i32 = 1;

/// Are ffmpeg and ffprobe both usable? Every test here needs them.
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

/// Skip the calling test, with a reason on stderr, when ffmpeg is missing.
/// A skipped test must be visible: a silent pass would mean this whole file
/// could stop testing anything without anyone noticing.
macro_rules! needs_ffmpeg {
    () => {
        if !media_tools_present() {
            eprintln!(
                "SKIP {}: ffmpeg and ffprobe are not both on PATH, so no test stream \
                 can be made",
                module_path!()
            );
            return;
        }
    };
}

/// A scratch directory of this test's own, under the system temp directory.
/// Left behind after the run, like the rest of this crate's tests, so a
/// failure can still be looked at.
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

/// The same stream with the frame timing left out, which is what a raw stream
/// coming from many other encoders looks like. ffprobe answers about this one
/// with its internal timebase, not a frame rate.
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

/// Run dex-sidecar and hand back everything it did.
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

/// Flip one byte in the middle of a file. Not the first byte: this is about
/// the hash noticing a change, not about anything in the stream's structure.
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
    let stream = dir.join("loop.265");
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
    // The size ffprobe reported travels along, informationally.
    let text = std::fs::read_to_string(&sidecar).unwrap();
    assert!(text.contains(r#""width":64"#), "sidecar: {text}");
    assert!(text.contains(r#""height":64"#), "sidecar: {text}");

    let checked = dex_sidecar(&["check", &sidecar, stream]);
    assert_eq!(exit_code(&checked), 0, "stderr: {}", stderr(&checked));
    assert!(stdout(&checked).starts_with("OK "), "{}", stdout(&checked));
}

/// The half that a writer nobody has tested always gets wrong: check has to
/// FAIL when the stream is not the one the sidecar was made from.
#[test]
fn check_fails_once_the_stream_changes() {
    needs_ffmpeg!();
    let dir = work_dir("corrupt");
    let stream = dir.join("loop.265");
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
    let stream = dir.join("loop.265");
    stream_with_timing(&stream);
    let stream = stream.to_str().unwrap();

    assert_eq!(exit_code(&dex_sidecar(&["write", stream, "--fps", "30"])), 0);

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

/// A typo in `--fps` binds the wrong rate into a sidecar that then verifies
/// perfectly forever, and the video plays at the wrong speed with nothing
/// reporting an error. So a rate far from the one in the stream is refused.
#[test]
fn an_fps_that_contradicts_the_stream_is_refused() {
    needs_ffmpeg!();
    let dir = work_dir("wrongfps");
    let stream = dir.join("loop.265");
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
        "--force must bind the rate that was asked for, not the detected one: {text}"
    );
}

/// A rate close to the detected one is two spellings of the same thing, not a
/// contradiction, and goes through without --force.
#[test]
fn an_fps_that_agrees_with_the_stream_needs_no_force() {
    needs_ffmpeg!();
    let dir = work_dir("agreeingfps");
    let stream = dir.join("loop.265");
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

/// When the encoder wrote no timing, there is nothing to read the rate from,
/// and guessing is the one thing this tool must never do.
#[test]
fn fps_is_required_when_the_stream_carries_no_timing() {
    needs_ffmpeg!();
    let dir = work_dir("notiming");
    let stream = dir.join("loop.265");
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

    // Told the rate, it writes one — and it still verifies.
    let written = dex_sidecar(&["write", stream, "--fps", "30"]);
    assert_eq!(exit_code(&written), 0, "stderr: {}", stderr(&written));
    let sidecar = format!("{stream}.json");
    let checked = dex_sidecar(&["check", &sidecar, stream]);
    assert_eq!(exit_code(&checked), 0, "stderr: {}", stderr(&checked));
}

#[test]
fn a_stream_that_is_not_there_is_reported_as_such() {
    let missing = work_dir("missing").join("nope.265");
    let out = dex_sidecar(&["write", missing.to_str().unwrap(), "--fps", "30"]);
    assert_eq!(exit_code(&out), EXIT_FAILED, "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("nope.265"),
        "the error must name the file: {}",
        stderr(&out)
    );
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
    // The old two-argument form, which is now `check`.
    let out = dex_sidecar(&["loop.265.json", "loop.265"]);
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
    let out = dex_sidecar(&["generate", "loop.265"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

/// A flag at the end of the line with nothing after it. It has to refuse
/// rather than quietly behave as if the flag were absent, which would write a
/// sidecar with a rate nobody chose.
#[test]
fn a_trailing_fps_with_no_value_gets_usage() {
    needs_ffmpeg!();
    let dir = work_dir("danglingfps");
    let stream = dir.join("loop.265");
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
    let out = dex_sidecar(&["write", "loop.265", "--out"]);
    assert_eq!(exit_code(&out), EXIT_REFUSED, "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

#[test]
fn an_fps_that_is_not_a_frame_rate_is_refused() {
    needs_ffmpeg!();
    let dir = work_dir("badfps");
    let stream = dir.join("loop.265");
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
