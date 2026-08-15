//! F3 — the asset+fps sidecar: parse, validate, and decide the binding.
//!
//! Why: the one failure that is undetectable BY CONSTRUCTION. A raw Annex-B
//! stream has no timestamps, so `--fps 25` on a 30 fps asset plays 20% slow,
//! forever, with zero errors and every metric nominal. The fix: the frame
//! rate travels WITH the asset (a sidecar written at ingest), bound by a
//! sha256 so a stale/wrong/truncated asset is refused at startup.
//!
//! Format — `<asset>.json` next to the asset (`loop.265` -> `loop.265.json`):
//!   {"fps":"30","sha256":"<64 hex>","width":3840,"height":2160,
//!    "source":"card.mp4","encoder_cmd":"ffmpeg ..."}
//! `fps` is a STRING, not a JSON number: "30000/1001" must survive exactly,
//! and 29.97 as a float invites drift. It is passed verbatim to mpv's
//! container-fps-override after grammar validation. Required: fps, sha256.
//! Optional, informational: width, height (integers), source, encoder_cmd
//! (strings). Unknown keys are ignored so ingest can add metadata without
//! breaking deployed players.
//!
//! The parser accepts a STRICT SUBSET of JSON — one flat object, string and
//! unsigned-integer values only (this applies uniformly to every key, known
//! or not: an ignored key's value must still be a string or unsigned
//! integer, never an array/bool/null/nested object). Anything else is a
//! parse error, and a parse error refuses startup. Fail-closed IS the F3
//! semantics: an unparseable sidecar and a missing one are the same
//! operational fact. Hand-rolled because the crate has a zero-dependency rule
//! and must build offline on the Pi.
//!
//! String escapes: \" \\ \/ \n \r \t, plus \uXXXX (standard JSON, including
//! UTF-16 surrogate pairs for code points above U+FFFF). \uXXXX support
//! matters beyond ordinary correctness: it is what every "safe by default"
//! JSON serializer reaches for on non-ASCII bytes -- Python's `json.dumps`
//! (default `ensure_ascii=True`) turns ANY non-ASCII character into \uXXXX,
//! and Go's `encoding/json` does the same for `<`, `>`, `&`. That includes
//! inside informational keys like `source`/`encoder_cmd` that this parser
//! does not even interpret -- refusing \uXXXX there refused otherwise
//! byte-perfect, correctly-hashed assets on nothing but a plausible ingest
//! tool's default serializer settings.

/// A parsed JSON value, restricted to the sidecar subset: strings and
/// unsigned integers only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Num(u64),
}

/// Parse `text` as a single flat JSON object under the sidecar subset grammar
/// (see module docs), returning its key/value pairs in source order.
pub fn parse_flat_json(text: &str) -> Result<Vec<(String, Value)>, String> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.skip_ws();
    p.expect(b'{')?;
    let mut out: Vec<(String, Value)> = Vec::new();
    p.skip_ws();
    if p.peek() == Some(b'}') {
        p.i += 1;
    } else {
        loop {
            p.skip_ws();
            let key = p.string()?;
            if out.iter().any(|(k, _)| *k == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            p.skip_ws();
            p.expect(b':')?;
            p.skip_ws();
            let val = match p.peek() {
                Some(b'"') => Value::Str(p.string()?),
                Some(c) if c.is_ascii_digit() => Value::Num(p.number()?),
                Some(c) => {
                    return Err(format!(
                        "unsupported value starting with {:?} (subset: strings and unsigned integers only)",
                        c as char
                    ))
                }
                None => return Err("unexpected end of input".into()),
            };
            out.push((key, val));
            p.skip_ws();
            match p.next_byte() {
                Some(b',') => continue,
                Some(b'}') => break,
                other => return Err(format!("expected ',' or '}}', got {other:?}")),
            }
        }
    }
    p.skip_ws();
    if p.i != p.b.len() {
        return Err("trailing data after closing '}'".into());
    }
    Ok(out)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn next_byte(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        match self.next_byte() {
            Some(g) if g == c => Ok(()),
            g => Err(format!("expected {:?}, got {g:?}", c as char)),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s: Vec<u8> = Vec::new();
        loop {
            match self.next_byte() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    return String::from_utf8(s)
                        .map_err(|_| String::from("invalid UTF-8 in string"))
                }
                Some(b'\\') => match self.next_byte() {
                    Some(b'"') => s.push(b'"'),
                    Some(b'\\') => s.push(b'\\'),
                    Some(b'/') => s.push(b'/'),
                    Some(b'n') => s.push(b'\n'),
                    Some(b'r') => s.push(b'\r'),
                    Some(b't') => s.push(b'\t'),
                    Some(b'u') => {
                        let ch = self.unicode_escape()?;
                        let mut buf = [0u8; 4];
                        s.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    e => {
                        return Err(format!(
                            r#"unsupported escape {e:?} (subset: \" \\ \/ \n \r \t \uXXXX)"#
                        ))
                    }
                },
                Some(c) if c < 0x20 => return Err("raw control character in string".into()),
                // Multibyte UTF-8 passes through byte-wise: continuation bytes
                // are >= 0x80, so they can never be mistaken for '"' or '\\'.
                Some(c) => s.push(c),
            }
        }
    }
    /// Decode a `\uXXXX` escape (the `u` is already consumed). JSON strings
    /// are UTF-16 code UNITS, so a code point above U+FFFF (most emoji, for
    /// instance) is always written as a surrogate PAIR -- two consecutive
    /// `\uXXXX` escapes -- per the JSON spec itself, not a sidecar-specific
    /// extension.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let unit = self.hex4()?;
        if (0xDC00..=0xDFFF).contains(&unit) {
            return Err(format!("lone low surrogate \\u{unit:04x} in string"));
        }
        if !(0xD800..=0xDBFF).contains(&unit) {
            // Not a surrogate: this code unit IS the code point, and every
            // non-surrogate u16 is a valid Unicode scalar value.
            return char::from_u32(u32::from(unit))
                .ok_or_else(|| format!("invalid \\u{unit:04x} escape"));
        }
        // High surrogate: must be immediately followed by a low surrogate.
        if self.next_byte() != Some(b'\\') || self.next_byte() != Some(b'u') {
            return Err(format!(
                "high surrogate \\u{unit:04x} not immediately followed by a low surrogate"
            ));
        }
        let low = self.hex4()?;
        if !(0xDC00..=0xDFFF).contains(&low) {
            return Err(format!(
                "high surrogate \\u{unit:04x} followed by \\u{low:04x}, which is not a low \
                 surrogate"
            ));
        }
        let c = 0x10000u32 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
        char::from_u32(c)
            .ok_or_else(|| format!("invalid surrogate pair \\u{unit:04x}\\u{low:04x}"))
    }
    fn hex4(&mut self) -> Result<u16, String> {
        let mut v: u16 = 0;
        for _ in 0..4 {
            let c = self
                .next_byte()
                .ok_or("unterminated \\u escape (need 4 hex digits)")?;
            let d = (c as char)
                .to_digit(16)
                .ok_or_else(|| format!("invalid hex digit {:?} in \\u escape", c as char))?;
            v = v * 16 + d as u16;
        }
        Ok(v)
    }
    fn number(&mut self) -> Result<u64, String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return Err("expected digits".into());
        }
        // Reject the rest of JSON's number grammar explicitly: the subset is
        // unsigned integers only (write fps as a string).
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err("floats are outside the sidecar subset (write fps as a string)".into());
        }
        std::str::from_utf8(&self.b[start..self.i])
            .unwrap()
            .parse::<u64>()
            .map_err(|e| format!("number out of range: {e}"))
    }
}

/// Is `s` a well-formed frame rate string: a positive integer ("30"), a
/// positive decimal ("29.97"), or a positive rational ("30000/1001")?
pub fn is_valid_fps(s: &str) -> bool {
    fn positive_int(t: &str) -> bool {
        !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && t.bytes().any(|b| b != b'0')
    }
    if let Some((num, den)) = s.split_once('/') {
        return positive_int(num) && positive_int(den);
    }
    if let Some((int, frac)) = s.split_once('.') {
        let digits_ok = !int.is_empty()
            && int.bytes().all(|b| b.is_ascii_digit())
            && !frac.is_empty()
            && frac.bytes().all(|b| b.is_ascii_digit());
        let nonzero = int.bytes().chain(frac.bytes()).any(|b| b != b'0');
        return digits_ok && nonzero;
    }
    positive_int(s)
}

/// A parsed, validated sidecar: the frame rate and the sha256 the asset must
/// match, plus optional informational dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    pub fps: String,
    pub sha256: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

impl Sidecar {
    /// Parse and validate a sidecar's JSON text. Fail-closed: any grammar
    /// violation, missing required key, invalid fps, or malformed sha256 is
    /// an error, on the theory that an unparseable sidecar and a missing one
    /// are the same operational fact.
    pub fn from_json(text: &str) -> Result<Sidecar, String> {
        let kv = parse_flat_json(text).map_err(|e| format!("sidecar JSON: {e}"))?;
        let mut fps = None;
        let mut sha = None;
        let mut width = None;
        let mut height = None;
        for (k, v) in kv {
            match (k.as_str(), v) {
                ("fps", Value::Str(s)) => fps = Some(s),
                ("fps", Value::Num(_)) => {
                    return Err(
                        "sidecar: fps must be a JSON STRING (\"30\", \"30000/1001\") \
                                so rational rates survive exactly"
                            .into(),
                    )
                }
                ("sha256", Value::Str(s)) => sha = Some(s),
                ("sha256", Value::Num(_)) => return Err("sidecar: sha256 must be a string".into()),
                ("width", Value::Num(n)) => width = Some(n),
                ("height", Value::Num(n)) => height = Some(n),
                ("width", Value::Str(_)) | ("height", Value::Str(_)) => {
                    return Err("sidecar: width/height must be integers".into())
                }
                // Unknown keys and informational strings: ignored, so ingest
                // can add metadata without breaking deployed players.
                _ => {}
            }
        }
        let fps = fps.ok_or("sidecar: missing required key \"fps\"")?;
        if !is_valid_fps(&fps) {
            return Err(format!(
                "sidecar: invalid fps {fps:?} (expect \"30\", \"29.97\" or \"30000/1001\")"
            ));
        }
        let sha = sha.ok_or("sidecar: missing required key \"sha256\"")?;
        let sha = sha.to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "sidecar: sha256 must be 64 hex digits, got {sha:?}"
            ));
        }
        Ok(Sidecar {
            fps,
            sha256: sha,
            width,
            height,
        })
    }
}

/// Where a bound fps value came from: the asset's sidecar, or the bench
/// escape hatch (`--bench-no-sidecar --fps <F>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsSource {
    Sidecar,
    BenchOverride,
}

/// Decide the fps to bind to, and where it came from.
///
/// Rules: with a sidecar present, its fps wins; an explicit `--fps` is
/// allowed only if it agrees with the sidecar (a mismatch is refused, naming
/// both values, since the sidecar is authoritative). With no sidecar, the
/// deploy path refuses to guess — the bench escape hatch is a deliberate,
/// two-flag act (`--bench-no-sidecar` AND `--fps`), never a silent fallback.
pub fn resolve_fps(
    sidecar_fps: Option<&str>,
    cli_fps: Option<&str>,
    bench_no_sidecar: bool,
) -> Result<(String, FpsSource), String> {
    if bench_no_sidecar {
        return match cli_fps {
            Some(f) if is_valid_fps(f) => Ok((f.to_string(), FpsSource::BenchOverride)),
            Some(f) => Err(format!("--fps {f:?} is not a valid frame rate")),
            None => Err("--bench-no-sidecar requires an explicit --fps".into()),
        };
    }
    match (sidecar_fps, cli_fps) {
        (Some(s), None) => Ok((s.to_string(), FpsSource::Sidecar)),
        (Some(s), Some(c)) if s == c => Ok((s.to_string(), FpsSource::Sidecar)),
        (Some(s), Some(c)) => Err(format!(
            "--fps {c} contradicts sidecar fps {s}; drop --fps (the sidecar is authoritative) \
             or fix the sidecar"
        )),
        (None, _) => Err(
            "no sidecar found; refusing to guess the frame rate. Re-ingest the asset to \
             produce <asset>.json, or use --bench-no-sidecar --fps <F> on a bench"
                .into(),
        ),
    }
}

/// Verify `payload`'s sha256 matches the sidecar's — the binding that refuses
/// a stale, wrong, or truncated asset at startup instead of playing it wrong
/// forever.
pub fn verify_payload(payload: &[u8], sidecar: &Sidecar) -> Result<(), String> {
    let actual = crate::sha256::sha256_hex(payload);
    if actual != sidecar.sha256 {
        return Err(format!(
            "asset does not match its sidecar: sha256 {actual} != sidecar {}; the asset or \
             sidecar is stale, wrong, or truncated — re-ingest",
            sidecar.sha256
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn parses_the_canonical_sidecar() {
        let text = format!(
            r#"{{"fps":"30","sha256":"{GOOD_SHA}","width":3840,"height":2160,"source":"card.mp4","encoder_cmd":"ffmpeg -i card.mp4 -c:v copy"}}"#
        );
        let s = Sidecar::from_json(&text).unwrap();
        assert_eq!(s.fps, "30");
        assert_eq!(s.sha256, GOOD_SHA);
        assert_eq!(s.width, Some(3840));
        assert_eq!(s.height, Some(2160));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = format!(r#"{{"fps":"30","sha256":"{GOOD_SHA}","future_key":"whatever"}}"#);
        assert!(Sidecar::from_json(&text).is_ok());
    }

    #[test]
    fn fps_as_number_is_refused_with_guidance() {
        let text = format!(r#"{{"fps":30,"sha256":"{GOOD_SHA}"}}"#);
        let e = Sidecar::from_json(&text).unwrap_err();
        assert!(e.contains("STRING"), "{e}");
    }

    #[test]
    fn missing_required_keys_are_refused_by_name() {
        let e = Sidecar::from_json(&format!(r#"{{"sha256":"{GOOD_SHA}"}}"#)).unwrap_err();
        assert!(e.contains("fps"), "{e}");
        let e = Sidecar::from_json(r#"{"fps":"30"}"#).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }

    #[test]
    fn bad_sha256_is_refused_uppercase_is_normalized() {
        for sha in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            let text = format!(r#"{{"fps":"30","sha256":"{sha}"}}"#);
            assert!(Sidecar::from_json(&text).is_err(), "sha {sha:?} accepted");
        }
        let text = format!(r#"{{"fps":"30","sha256":"{}"}}"#, GOOD_SHA.to_uppercase());
        assert_eq!(Sidecar::from_json(&text).unwrap().sha256, GOOD_SHA);
    }

    #[test]
    fn fps_grammar() {
        for ok in ["30", "25", "29.97", "23.976", "30000/1001", "60"] {
            assert!(is_valid_fps(ok), "{ok} should be valid");
        }
        for bad in [
            "", "0", "00", "0/30", "30/0", "-30", "+30", " 30", "30 ", "30/", "/1001", "1e3",
            "29.97.5", "29.", ".97", "banana", "0.0",
        ] {
            assert!(!is_valid_fps(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn parser_rejects_everything_outside_the_subset() {
        for bad in [
            "",                               // no object
            "[1,2]",                          // array at top level
            r#"{"a":{"b":1}}"#,               // nested object
            r#"{"a":[1]}"#,                   // array value
            r#"{"a":true}"#,                  // boolean
            r#"{"a":null}"#,                  // null
            r#"{"a":-1}"#,                    // negative number
            r#"{"a":1.5}"#,                   // float
            r#"{"a":1e3}"#,                   // exponent
            r#"{"a":"x"}"trailing"#,          // trailing data
            r#"{"a":"x""b":"y"}"#,            // missing comma
            r#"{"a":"unterminated}"#,         // unterminated string
            r#"{"a":"x","a":"y"}"#,           // duplicate key
        ] {
            assert!(parse_flat_json(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn unicode_escapes_are_decoded() {
        // These are raw string literals: the JSON *source text* the parser
        // receives contains the literal four characters `\`, `u`, and four
        // hex digits -- the parser itself must turn that into a code point.
        // The expected side uses Rust's OWN (unrelated) `\u{...}` syntax
        // purely so this file's source stays plain ASCII.

        // ASCII code point spelled via the escape A.
        let json = format!(r#"{{"a":"\{}0041"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("A".to_string()));

        // The actual field bug: json.dumps({"source": "Zürich.mp4"}) with
        // Python's DEFAULT ensure_ascii=True produces the escape ü for
        // "ü". Built with `format!` so this file's source stays plain ASCII;
        // `\u{fc}` on the expected side is Rust's own (unrelated) escape.
        let json = format!(r#"{{"a":"Z\{}00fcrich.mp4"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("Z\u{fc}rich.mp4".to_string()));

        // Outside the BMP: a UTF-16 surrogate pair, standard JSON (not a
        // sidecar-specific extension) -- U+1F389 PARTY POPPER is
        // 🎉.
        let json = format!(r#"{{"a":"\{0}d83c\{0}df89"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("\u{1F389}".to_string()));

        // A \u escape next to a plain escape in the same string, to prove
        // they compose: é (é) then a plain \n.
        let json = format!(r#"{{"a":"caf\{}00e9\nmore"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("caf\u{e9}\nmore".to_string()));
    }

    #[test]
    fn malformed_unicode_escapes_are_refused() {
        for bad in [
            r#"{"a":"\u12"}"#,         // truncated: only 2 hex digits
            r#"{"a":"\u12zz"}"#,       // non-hex digits
            r#"{"a":"\ud800"}"#,       // lone high surrogate, no pair follows
            r#"{"a":"\udc00"}"#,       // lone low surrogate
            r#"{"a":"\ud800A"}"#,      // high surrogate followed by a non-surrogate
            r#"{"a":"\ud800\udbff"}"#, // high surrogate followed by ANOTHER high surrogate
        ] {
            assert!(parse_flat_json(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn unicode_escape_in_an_ignored_sidecar_key_no_longer_breaks_startup() {
        // The concrete field scenario: an ingest tool's default-safe JSON
        // serializer \u-escapes a non-ASCII byte inside "source", a key this
        // player does not even interpret -- and startup used to refuse
        // anyway, on an otherwise byte-perfect, correctly-hashed asset. Built
        // with `format!` so the JSON text itself contains the literal escape
        // ü, not an already-decoded byte -- that is the actual bug.
        let text = format!(
            r#"{{"fps":"30","sha256":"{GOOD_SHA}","source":"Z\{}00fcrich.mp4"}}"#,
            'u'
        );
        let s = Sidecar::from_json(&text).unwrap();
        assert_eq!(s.fps, "30");
    }

    #[test]
    fn parser_accepts_the_subset() {
        let kv = parse_flat_json(r#" { "a" : "x\n\"q\"" , "n" : 42 } "#).unwrap();
        assert_eq!(
            kv,
            vec![
                ("a".to_string(), Value::Str("x\n\"q\"".to_string())),
                ("n".to_string(), Value::Num(42)),
            ]
        );
        assert_eq!(parse_flat_json("{}").unwrap(), vec![]);
    }

    #[test]
    fn deploy_path_takes_fps_from_sidecar() {
        assert_eq!(
            resolve_fps(Some("30"), None, false).unwrap(),
            ("30".to_string(), FpsSource::Sidecar)
        );
    }

    #[test]
    fn agreeing_cli_fps_allowed_disagreeing_refused_naming_both() {
        assert!(resolve_fps(Some("30"), Some("30"), false).is_ok());
        let e = resolve_fps(Some("30"), Some("25"), false).unwrap_err();
        assert!(e.contains("30") && e.contains("25"), "{e}");
    }

    #[test]
    fn missing_sidecar_is_refused_without_the_bench_flag() {
        let e = resolve_fps(None, Some("30"), false).unwrap_err();
        assert!(e.contains("bench"), "{e}");
        assert!(resolve_fps(None, None, false).is_err());
    }

    #[test]
    fn bench_escape_hatch_requires_both_flags_and_a_valid_rate() {
        assert_eq!(
            resolve_fps(None, Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        // bench flag with a sidecar present: the sidecar is IGNORED — that is
        // what "bench" means — and the CLI value wins.
        assert_eq!(
            resolve_fps(Some("25"), Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        assert!(resolve_fps(None, None, true).is_err());
        assert!(resolve_fps(None, Some("banana"), true).is_err());
    }

    #[test]
    fn verify_payload_binds_bytes_to_sidecar() {
        let payload = b"the asset bytes";
        let s = Sidecar {
            fps: "30".into(),
            sha256: crate::sha256::sha256_hex(payload),
            width: None,
            height: None,
        };
        assert!(verify_payload(payload, &s).is_ok());
        let bad = Sidecar {
            fps: "30".into(),
            sha256: "a".repeat(64),
            width: None,
            height: None,
        };
        let e = verify_payload(payload, &bad).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }
}
