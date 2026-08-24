//! Create and verify the sidecar file that sits next to every video dexd
//! plays.
//!
//! A raw HEVC stream carries no frame rate, so the rate travels beside it in
//! `<stream>.json`, a small JSON file holding the frame rate and the SHA-256
//! of the stream's exact bytes. dexd refuses to start when that file is
//! missing, unreadable, or bound to different bytes.
//!
//! `write` creates a sidecar and `check` verifies an existing pair. Both call
//! the parser and the hash from the library the player links, so the sidecar
//! format has one implementation. `USAGE` below gives the options and the exit
//! codes.
//!
//! See docs/design/sidecar.md#dex-sidecar.

use dexd::sha256::sha256_hex;
use dexd::sidecar;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Refused before anything was written, or a malformed command line.
const EXIT_REFUSED: u8 = 2;
/// The stream could not be read, or a sidecar failed verification.
const EXIT_FAILED: u8 = 1;

const USAGE: &str = "\
usage: dex-sidecar check <sidecar.json> <stream.265>
       dex-sidecar write <stream.265> [--fps <F>] [--out <file>] [--force]

dexd will not play a raw HEVC stream without a sidecar next to it: a small
JSON file holding the frame rate and the SHA-256 of the stream's exact bytes.

check <sidecar.json> <stream.265>
    Parse the sidecar and confirm its SHA-256 matches the stream on disk.

write <stream.265>
    --fps <F>   Frame rate to bind, written as \"30\", \"29.97\" or
                \"30000/1001\". Without it the rate is read from the stream
                with ffprobe, which only works when the encoder wrote timing
                into the stream itself. When it did not, --fps is required
                rather than guessed.
    --out <file>
                Where to write the sidecar. Default <stream.265>.json, which
                is the only name dexd looks for.
    --force     Proceed past two refusals: overwriting a sidecar that already
                exists, and a --fps that disagrees with the rate ffprobe read
                from the stream. A wrong frame rate plays the video at the
                wrong speed for as long as it runs, without any error, so
                both are refused unless you say otherwise.

exit codes: 0 ok
            1 the stream could not be read, or verification failed
            2 bad command line, or a refusal
";

fn usage() -> ExitCode {
    eprint!("{USAGE}");
    ExitCode::from(EXIT_REFUSED)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => cmd_check(&args[1..]),
        Some("write") => cmd_write(&args[1..]),
        _ => usage(),
    }
}

// ---- check ---------------------------------------------------------------

/// Verify an existing sidecar against its stream.
fn cmd_check(args: &[String]) -> ExitCode {
    let [sidecar_path, stream_path] = args else {
        return usage();
    };

    let text = match fs::read_to_string(sidecar_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {sidecar_path}: {e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };

    let parsed = match sidecar::Sidecar::from_json(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {sidecar_path}: {e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };

    // The same `fs::read` call the player makes in main.rs: the digest has to
    // cover the bytes the player would see. Change both together or neither.
    let payload = match fs::read(stream_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot read {stream_path}: {e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };

    if let Err(e) = sidecar::verify_payload(&payload, &parsed) {
        eprintln!("error: {e}");
        return ExitCode::from(EXIT_FAILED);
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

// ---- frame rates ---------------------------------------------------------

// A rate read from the stream must fall in this range. Without timing in the
// stream's own headers ffprobe answers with its internal timebase, typically
// 1200000/1, which is not a frame rate. Before narrowing the range, check that
// it still passes every camera and encoder rate in use — see
// docs/design/sidecar.md#frame-rate-at-ingest.
const SANE_MIN: f64 = 1.0;
const SANE_MAX: f64 = 1000.0;

/// How far an explicit `--fps` may sit from a rate read out of the stream
/// before the two are treated as contradicting each other.
const FPS_TOLERANCE: f64 = 0.02;

/// Turn ffprobe's `r_frame_rate` into a frame rate reduced to lowest terms.
/// `None` when the value is not a `num/den` fraction of non-zero whole numbers,
/// or falls outside `SANE_MIN..=SANE_MAX`.
fn detect_fps(rate: &str) -> Option<String> {
    let parts: Vec<&str> = rate.trim().split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let num: u64 = parts[0].parse().ok()?;
    let den: u64 = parts[1].parse().ok()?;
    if num == 0 || den == 0 {
        return None;
    }
    let value = num as f64 / den as f64;
    if !(SANE_MIN..=SANE_MAX).contains(&value) {
        return None;
    }
    let g = gcd(num, den);
    let (n, d) = (num / g, den / g);
    Some(if d == 1 {
        n.to_string()
    } else {
        format!("{n}/{d}")
    })
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// A frame rate as a number, so two of them can be compared. The result is
/// rounded to six decimals, far finer than `FPS_TOLERANCE`, so that `30000/1001`
/// and `29.97` do not differ by a floating-point artifact.
fn fps_to_decimal(fps: &str) -> Option<f64> {
    let value = if let Some((num, den)) = fps.split_once('/') {
        let num: f64 = num.parse().ok()?;
        let den: f64 = den.parse().ok()?;
        if den == 0.0 {
            return None;
        }
        num / den
    } else {
        fps.parse().ok()?
    };
    if !value.is_finite() {
        return None;
    }
    Some((value * 1e6).round() / 1e6)
}

/// Whether two frame rates contradict each other: two spellings of one rate
/// (`30000/1001` and `29.97`) do not, `3` and `30` do. A value that cannot be
/// read as a number counts as a contradiction.
fn fps_disagrees(given: &str, detected: &str) -> bool {
    match (fps_to_decimal(given), fps_to_decimal(detected)) {
        (Some(a), Some(b)) => (a - b).abs() > FPS_TOLERANCE,
        _ => true,
    }
}

/// Whether dexd's parser reads this frame rate back unchanged, asked by building
/// a sidecar around the value and parsing it. The rate format has one
/// implementation, the player's — see docs/design/sidecar.md#frame-rate-at-ingest.
fn fps_is_acceptable(fps: &str) -> bool {
    let probe = sidecar_json(fps, &"0".repeat(64), None, None);
    match sidecar::Sidecar::from_json(&probe) {
        // Compare as well as parse: a value carrying quotes or backslashes
        // would otherwise reshape the JSON built around it.
        Ok(parsed) => parsed.fps == fps,
        Err(_) => false,
    }
}

// ---- ffprobe -------------------------------------------------------------

/// What ffprobe could say about the stream. Every field is optional: ffprobe
/// may be absent, may fail on a raw stream, and may answer `N/A`.
struct Probe {
    width: Option<u64>,
    height: Option<u64>,
    rate: Option<String>,
}

/// Ask ffprobe for the stream's size and frame rate. `None` when ffprobe is not
/// installed or could not read the file; the caller then needs `--fps`.
fn probe_stream(path: &Path) -> Option<Probe> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let mut fields = line.splitn(3, ',');
    let width = fields.next().unwrap_or("");
    let height = fields.next().unwrap_or("");
    let rate = fields.next().unwrap_or("").trim();
    Some(Probe {
        width: parse_dimension(width),
        height: parse_dimension(height),
        rate: (!rate.is_empty()).then(|| rate.to_string()),
    })
}

/// A width or height from ffprobe. `N/A`, empty, and anything that is not a
/// plain number all mean "not available".
fn parse_dimension(field: &str) -> Option<u64> {
    let field = field.trim();
    if field.is_empty() || field == "N/A" {
        return None;
    }
    field.parse().ok()
}

// ---- write ---------------------------------------------------------------

/// One line of JSON, in the shape dexd reads. Width and height are optional
/// and only ever informational — the player never looks at them.
fn sidecar_json(fps: &str, sha256: &str, width: Option<u64>, height: Option<u64>) -> String {
    let mut out = format!(r#"{{"fps":"{fps}","sha256":"{sha256}""#);
    if let (Some(w), Some(h)) = (width, height) {
        out.push_str(&format!(r#","width":{w},"height":{h}"#));
    }
    out.push_str("}\n");
    out
}

/// Command line for `write`, once it has been read successfully.
struct WriteArgs {
    stream: String,
    fps: Option<String>,
    out: Option<String>,
    force: bool,
}

/// Read `write`'s arguments. `None` means the command line was malformed and
/// the caller should print usage. A flag whose value is missing, such as the
/// last token on the line, makes the whole command line malformed.
fn parse_write_args(args: &[String]) -> Option<WriteArgs> {
    let mut stream: Option<String> = None;
    let mut fps: Option<String> = None;
    let mut out: Option<String> = None;
    let mut force = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fps" => {
                fps = Some(args.get(i + 1)?.clone());
                i += 2;
            }
            "--out" => {
                out = Some(args.get(i + 1)?.clone());
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            flag if flag.starts_with('-') => return None,
            positional => {
                if stream.is_some() {
                    return None;
                }
                stream = Some(positional.to_string());
                i += 1;
            }
        }
    }

    Some(WriteArgs {
        stream: stream?,
        fps,
        out,
        force,
    })
}

fn cmd_write(args: &[String]) -> ExitCode {
    let Some(args) = parse_write_args(args) else {
        return usage();
    };
    let stream = PathBuf::from(&args.stream);

    match fs::metadata(&stream) {
        Ok(m) if !m.is_file() => {
            eprintln!("error: not a file: {}", args.stream);
            return ExitCode::from(EXIT_FAILED);
        }
        Ok(m) if m.len() == 0 => {
            eprintln!("error: {} is empty", args.stream);
            return ExitCode::from(EXIT_FAILED);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", args.stream);
            return ExitCode::from(EXIT_FAILED);
        }
    }

    let out = args
        .out
        .clone()
        .unwrap_or_else(|| format!("{}.json", args.stream));

    // Replacing an existing sidecar takes --force: the old one may be the only
    // record of the rate the video was prepared with.
    if Path::new(&out).exists() && !args.force {
        eprintln!("error: {out} already exists; pass --force to replace it");
        return ExitCode::from(EXIT_REFUSED);
    }

    if let Some(given) = &args.fps {
        if !fps_is_acceptable(given) {
            eprintln!(
                "error: --fps {given:?} is not a frame rate dexd can read; \
                 write it as \"30\", \"29.97\" or \"30000/1001\""
            );
            return ExitCode::from(EXIT_REFUSED);
        }
    }

    let probe = probe_stream(&stream);
    let detected = probe
        .as_ref()
        .and_then(|p| p.rate.as_deref())
        .and_then(detect_fps);

    let fps = match (&args.fps, &detected) {
        (Some(given), Some(detected)) if fps_disagrees(given, detected) => {
            if !args.force {
                eprintln!(
                    "error: --fps {given} disagrees with {detected}, the rate ffprobe read \
                     from {}.",
                    args.stream
                );
                eprintln!(
                    "       Refusing rather than picking one: at the wrong rate the video \
                     plays too fast or too slow for as long as it runs, and nothing \
                     reports an error."
                );
                eprintln!(
                    "       Pass --force if {given} is right and ffprobe is wrong, or drop \
                     --fps to use {detected}."
                );
                return ExitCode::from(EXIT_REFUSED);
            }
            eprintln!(
                "warning: --fps {given} disagrees with {detected}, the rate ffprobe read \
                 from {}; using {given} because --force was given.",
                args.stream
            );
            given.clone()
        }
        (Some(given), _) => given.clone(),
        (None, Some(detected)) => {
            eprintln!(
                "note: frame rate {detected} read from {}'s own headers. Pass --fps if you \
                 know the rate from somewhere else.",
                args.stream
            );
            detected.clone()
        }
        (None, None) => {
            eprintln!(
                "error: no frame rate for {}, and none could be read from the stream.",
                args.stream
            );
            eprintln!(
                "       A raw stream has no timestamps, so this only works when the \
                 encoder wrote timing into the stream itself; here ffprobe is missing, \
                 could not read the file, or answered with something that is not a \
                 frame rate."
            );
            eprintln!("       Pass --fps <F> explicitly.");
            return ExitCode::from(EXIT_REFUSED);
        }
    };

    // The same `fs::read` call the player makes, so both sides hash the same
    // bytes.
    let payload = match fs::read(&stream) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", args.stream);
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let sha256 = sha256_hex(&payload);

    let (width, height) = match &probe {
        Some(p) => (p.width, p.height),
        None => (None, None),
    };
    if width.is_none() || height.is_none() {
        eprintln!(
            "note: ffprobe did not report the video size; leaving width and height out \
             (both are informational, dexd never reads them)."
        );
    }

    let json = sidecar_json(&fps, &sha256, width, height);

    // Parse the text with the player's parser and re-check the digest before
    // the file takes its final name, so a defect here writes nothing. See
    // docs/design/sidecar.md#writing.
    let parsed = match sidecar::Sidecar::from_json(&json) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: the sidecar just built was rejected by dexd's own parser: {e}");
            eprintln!("       This is a bug in dex-sidecar, not in your input. {out} not written.");
            return ExitCode::from(EXIT_FAILED);
        }
    };
    if parsed.fps != fps || parsed.sha256 != sha256 {
        eprintln!(
            "error: the sidecar just built reads back as fps {} sha256 {}, not fps {fps} \
             sha256 {sha256}.",
            parsed.fps, parsed.sha256
        );
        eprintln!("       This is a bug in dex-sidecar, not in your input. {out} not written.");
        return ExitCode::from(EXIT_FAILED);
    }
    if let Err(e) = sidecar::verify_payload(&payload, &parsed) {
        eprintln!("error: {e}");
        eprintln!("       {out} not written.");
        return ExitCode::from(EXIT_FAILED);
    }

    if let Err(e) = write_atomically(Path::new(&out), &json) {
        eprintln!("error: cannot write {out}: {e}");
        return ExitCode::from(EXIT_FAILED);
    }

    eprintln!("wrote {out}");
    print!("{json}");
    ExitCode::SUCCESS
}

/// Write via a temporary file in the same directory and rename over the
/// target, so an interrupted run cannot leave half a sidecar where dexd will
/// look for a whole one.
fn write_atomically(out: &Path, contents: &str) -> std::io::Result<()> {
    let mut tmp = out.as_os_str().to_os_string();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    // Whatever fails, no partial temp file is left beside the stream.
    if let Err(e) = fs::write(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, out) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_in_range_is_reduced_to_lowest_terms() {
        assert_eq!(detect_fps("30/1").as_deref(), Some("30"));
        assert_eq!(detect_fps("60/2").as_deref(), Some("30"));
        assert_eq!(detect_fps("30000/1001").as_deref(), Some("30000/1001"));
        assert_eq!(detect_fps("2997/125").as_deref(), Some("2997/125"));
        assert_eq!(detect_fps("24000/1001").as_deref(), Some("24000/1001"));
    }

    /// With no timing in the stream's headers, ffprobe answers with its
    /// internal timebase, which is not a frame rate.
    #[test]
    fn ffprobes_internal_timebase_is_out_of_range() {
        assert_eq!(detect_fps("1200000/1"), None);
    }

    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        assert_eq!(detect_fps("1/1").as_deref(), Some("1"));
        assert_eq!(detect_fps("1000/1").as_deref(), Some("1000"));
        assert_eq!(detect_fps("1/2"), None); // 0.5, below the floor
        assert_eq!(detect_fps("1001/1"), None); // just over the ceiling
    }

    #[test]
    fn a_rate_that_is_not_a_fraction_is_refused() {
        for bad in [
            "", "30", "0/0", "0/1", "30/0", "-30/1", "30/-1", "30/1/1", "banana", "N/A", "1.5/1",
        ] {
            assert_eq!(detect_fps(bad), None, "read {bad:?} as a rate");
        }
    }

    #[test]
    fn the_same_rate_spelled_two_ways_does_not_count_as_a_disagreement() {
        assert!(!fps_disagrees("30000/1001", "29.97"));
        assert!(!fps_disagrees("30", "30/1"));
        assert!(!fps_disagrees("29.97", "2997/100"));
    }

    #[test]
    fn a_typo_counts_as_a_disagreement() {
        assert!(fps_disagrees("3", "30"));
        assert!(fps_disagrees("25", "30"));
        assert!(fps_disagrees("24", "23.976"));
    }

    /// The tolerance is about two spellings of one rate, not about two rates
    /// that happen to be close.
    #[test]
    fn the_tolerance_is_narrow() {
        assert!(!fps_disagrees("30.00", "30.019"));
        assert!(fps_disagrees("30.00", "30.021"));
    }

    #[test]
    fn an_uncomparable_rate_counts_as_a_disagreement() {
        assert!(fps_disagrees("banana", "30"));
        assert!(fps_disagrees("30", "banana"));
        assert!(fps_disagrees("30/0", "30"));
    }

    #[test]
    fn only_the_rates_dexd_reads_are_accepted() {
        for good in ["30", "25", "29.97", "23.976", "30000/1001", "60"] {
            assert!(fps_is_acceptable(good), "rejected {good:?}");
        }
        for bad in ["", "0", "-30", "banana", "30 ", " 30", "1e3", "30/", "29."] {
            assert!(!fps_is_acceptable(bad), "accepted {bad:?}");
        }
    }

    /// A rate carrying JSON punctuation must not reshape the file built around
    /// it: what comes back out has to be what went in.
    #[test]
    fn a_rate_carrying_json_punctuation_is_refused() {
        for hostile in [
            r#"30","sha256":"0000000000000000000000000000000000000000000000000000000000000000"#,
            r#"30\"#,
            "30\"",
            "30\n",
        ] {
            assert!(!fps_is_acceptable(hostile), "accepted {hostile:?}");
        }
    }

    #[test]
    fn the_written_shape_is_one_line_with_optional_size() {
        let sha = "a".repeat(64);
        assert_eq!(
            sidecar_json("30", &sha, Some(3840), Some(2160)),
            format!("{{\"fps\":\"30\",\"sha256\":\"{sha}\",\"width\":3840,\"height\":2160}}\n")
        );
        assert_eq!(
            sidecar_json("30000/1001", &sha, None, None),
            format!("{{\"fps\":\"30000/1001\",\"sha256\":\"{sha}\"}}\n")
        );
        // A size known on only one axis is left out entirely.
        assert_eq!(
            sidecar_json("30", &sha, Some(3840), None),
            format!("{{\"fps\":\"30\",\"sha256\":\"{sha}\"}}\n")
        );
    }

    #[test]
    fn what_is_written_parses_back_as_what_went_in() {
        let sha = "b".repeat(64);
        let json = sidecar_json("30000/1001", &sha, Some(3840), Some(2160));
        let parsed = sidecar::Sidecar::from_json(&json).unwrap();
        assert_eq!(parsed.fps, "30000/1001");
        assert_eq!(parsed.sha256, sha);
        assert_eq!(parsed.width, Some(3840));
        assert_eq!(parsed.height, Some(2160));
    }

    #[test]
    fn a_missing_flag_value_is_a_malformed_command_line() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_write_args(&args(&["artwork.265", "--fps"])).is_none());
        assert!(parse_write_args(&args(&["artwork.265", "--out"])).is_none());
        assert!(parse_write_args(&args(&[])).is_none());
        assert!(parse_write_args(&args(&["--fps", "30"])).is_none());
        assert!(parse_write_args(&args(&["a.265", "b.265"])).is_none());
        assert!(parse_write_args(&args(&["artwork.265", "--nope"])).is_none());
    }

    #[test]
    fn flags_are_read_in_any_order() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = parse_write_args(&args(&[
            "--force", "--fps", "30", "artwork.265", "--out", "s.json",
        ]))
        .unwrap();
        assert_eq!(a.stream, "artwork.265");
        assert_eq!(a.fps.as_deref(), Some("30"));
        assert_eq!(a.out.as_deref(), Some("s.json"));
        assert!(a.force);

        let a = parse_write_args(&args(&["artwork.265"])).unwrap();
        assert_eq!(a.stream, "artwork.265");
        assert!(a.fps.is_none());
        assert!(a.out.is_none());
        assert!(!a.force);
    }

    #[test]
    fn a_size_ffprobe_could_not_report_is_left_out() {
        assert_eq!(parse_dimension("3840"), Some(3840));
        assert_eq!(parse_dimension(" 2160 "), Some(2160));
        assert_eq!(parse_dimension("N/A"), None);
        assert_eq!(parse_dimension(""), None);
        assert_eq!(parse_dimension("-1"), None);
        assert_eq!(parse_dimension("1920.0"), None);
    }
}
