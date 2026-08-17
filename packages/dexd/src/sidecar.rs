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
//! operational fact.
//!
//! The subset is enforced by a serde visitor over `serde_json` (SPEC §5c); it
//! was hand-rolled while the crate had a zero-dependency rule. Exactly ONE
//! rule survives as our own code, because it is the one a `Map` cannot state:
//! **duplicate keys are rejected** rather than silently last-wins.
//!
//! String escapes (\" \\ \/ \n \r \t, and \uXXXX including UTF-16 surrogate
//! pairs) are serde_json's problem now — but they are recorded here as a
//! REQUIREMENT, because the tempting "simplification" is to ban non-ASCII and
//! it would be wrong. \uXXXX is what every "safe by default" JSON serializer
//! reaches for: Python's `json.dumps` (default `ensure_ascii=True`) turns ANY
//! non-ASCII character into \uXXXX, and Go's `encoding/json` does the same for
//! `<`, `>`, `&`. That includes informational keys like `source`/`encoder_cmd`
//! which are never interpreted here — a `source` filename may legitimately be
//! "Karte–Süd.mp4". Refusing escapes would refuse byte-perfect, correctly
//! hashed assets over nothing but an ingest tool's serializer settings.
//! (`fps` and `sha256` are ASCII by their own grammars, validated below.)

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
    serde_json::from_str::<FlatObject>(text)
        .map(|f| f.0)
        .map_err(|e| e.to_string())
}

/// A newtype whose `Deserialize` impl *is* the subset grammar.
///
/// Hand-written visitor rather than `#[derive]` or `serde_json::Map`, for one
/// reason: **the grammar rejects duplicate keys and a Map cannot express
/// that** — it silently keeps the last. `{"fps":"30","fps":"25"}` has to be an
/// error rather than a coin flip decided by which parser reads it, because F3
/// is a fail-closed gate: an ambiguous sidecar and a missing one are the same
/// operational fact. Everything else the old hand-rolled parser did — lexing,
/// escapes, surrogate pairs, trailing-data and structural errors — is
/// serde_json's now.
struct FlatObject(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for FlatObject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // deserialize_map, not deserialize_any: a top-level array or scalar is
        // rejected by serde_json with a type error before we see it.
        d.deserialize_map(FlatObjectVisitor)
    }
}

struct FlatObjectVisitor;

/// Name a rejected value's type for the error message. Kept exhaustive rather
/// than `_ =>` so a future serde_json variant is a compile error here.
fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "a nested object",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Number(_) => "a number",
    }
}

impl<'de> serde::de::Visitor<'de> for FlatObjectVisitor {
    type Value = FlatObject;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a flat JSON object whose values are strings or unsigned integers")
    }

    fn visit_map<A>(self, mut map: A) -> Result<FlatObject, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        use serde::de::Error as _;
        let mut out: Vec<(String, Value)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.iter().any(|(k, _)| *k == key) {
                return Err(A::Error::custom(format!("duplicate key {key:?}")));
            }
            // The subset applies UNIFORMLY, to unknown keys too: an ignored
            // key's value must still be a string or unsigned integer. Reading
            // into serde_json::Value first is what lets us say so — and lets
            // an informational key like `source` hold any text, escapes and
            // all, which is the behaviour the format actually needs.
            let value = match map.next_value::<serde_json::Value>()? {
                serde_json::Value::String(s) => Value::Str(s),
                serde_json::Value::Number(n) => Value::Num(n.as_u64().ok_or_else(|| {
                    A::Error::custom(format!(
                        "value for {key:?} must be an unsigned integer, got {n} \
                         (floats and negatives are outside the sidecar subset; \
                         write fps as a string)"
                    ))
                })?),
                other => {
                    return Err(A::Error::custom(format!(
                        "unsupported value for {key:?}: {} (subset: strings and \
                         unsigned integers only)",
                        type_name(&other)
                    )))
                }
            };
            out.push((key, value));
        }
        Ok(FlatObject(out))
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

    // The other half of the escape requirement, and the half that is easier to
    // lose: a serializer that does NOT escape (jq, Rust's own serde_json,
    // Python with ensure_ascii=False) writes the character as raw UTF-8 bytes.
    // Both spellings mean the same sidecar and both must parse.
    //
    // This exists because "no sidecar value can hold a non-ASCII byte, so
    // reject non-ASCII at the door" was proposed during the SPEC §5c work and
    // is WRONG: it is true of `fps` and `sha256`, and false of `source` --
    // asset filenames are routinely not ASCII. Narrowing the contract there
    // would have refused correctly-hashed assets over their filename.
    #[test]
    fn raw_utf8_in_an_informational_key_parses_like_its_escaped_form() {
        // Built with char escapes so this file's source stays plain ASCII.
        let raw = "Z\u{fc}rich.mp4".to_string();
        let json = format!(r#"{{"source":"{raw}"}}"#);
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str(raw.clone()));

        // ... and is indistinguishable from the \u-escaped spelling.
        let escaped = format!(r#"{{"source":"Z\{}00fcrich.mp4"}}"#, 'u');
        assert_eq!(parse_flat_json(&escaped).unwrap(), kv);

        // A full sidecar with a non-ASCII source must still bind normally --
        // `source` is informational and never interpreted.
        let text = format!(
            r#"{{"fps":"30","sha256":"{GOOD_SHA}","source":"{raw}"}}"#
        );
        let s = Sidecar::from_json(&text).unwrap();
        assert_eq!(s.fps, "30");
    }

    // Duplicate rejection is the ONE grammar rule still implemented by hand
    // (SPEC §5c): serde_json's Map silently keeps the last value, so the
    // visitor has to say so itself. Tested by name rather than only inside the
    // reject-everything batch, because a refactor that dropped the visitor for
    // a plain Map would still pass every other test in this file.
    #[test]
    fn duplicate_keys_are_refused_and_named() {
        let e = parse_flat_json(r#"{"fps":"30","fps":"25"}"#).unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
        assert!(e.contains("fps"), "{e}");

        // Including a duplicated key the sidecar does not interpret: the rule
        // is about the document being unambiguous, not about which keys matter.
        let e = parse_flat_json(r#"{"source":"a","source":"b"}"#).unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
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
