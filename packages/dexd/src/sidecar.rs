//! The sidecar: parse `<asset>.json`, validate it, and decide the frame rate.
//!
//! A raw Annex-B stream has no frame rate in it, so the rate is stored beside
//! the asset in a sidecar file and bound to it by a sha256 that startup
//! re-checks. See docs/design/sidecar.md#asset-binding.
//!
//! Format — `<asset>.json` next to the asset (`loop.265` -> `loop.265.json`):
//!   {"fps":"30","sha256":"<64 hex>","width":3840,"height":2160,
//!    "source":"card.mp4","encoder_cmd":"ffmpeg ..."}
//!
//! Required: fps, sha256. Optional and informational: width, height
//! (integers), source, encoder_cmd (strings). Unknown keys are ignored, so a
//! preparation tool can add metadata without breaking deployed players. `fps`
//! is a JSON string so a rational rate such as "30000/1001" survives
//! unchanged; it reaches mpv's container-fps-override verbatim.
//!
//! The grammar is a subset of JSON: one flat object whose values are strings
//! or unsigned integers, for every key, known or not. Values carry any text,
//! escapes and raw UTF-8 alike; do not narrow them. Anything outside the
//! subset is a parse error, and a parse error refuses startup.

/// A parsed JSON value under the sidecar subset: a string or an unsigned
/// integer.
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

/// A newtype whose `Deserialize` impl is the subset grammar.
///
/// The map visitor is hand-written because `serde_json::Map` keeps the last of
/// two same-named keys and reports no error, so it cannot state the
/// duplicate-key rule. Lexing, escapes, surrogate pairs, trailing data and
/// structural errors are serde_json's.
/// See docs/design/sidecar.md#sidecar-grammar.
struct FlatObject(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for FlatObject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // deserialize_map, not deserialize_any: serde_json then rejects a
        // top-level array or scalar with a type error before the visitor runs.
        d.deserialize_map(FlatObjectVisitor)
    }
}

struct FlatObjectVisitor;

/// Name a rejected value's type for the error message. The match is
/// exhaustive, so a new serde_json variant becomes a compile error here.
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
            // Read into serde_json::Value first: that applies the subset to
            // unknown keys as well, and lets an informational key such as
            // `source` carry any text, escapes included.
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

/// Whether `s` is a well-formed frame rate: a positive integer ("30"), a
/// positive decimal ("29.97") or a positive rational ("30000/1001").
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
    /// Parse and validate a sidecar's JSON text. A grammar violation, a
    /// missing required key, an invalid fps or a malformed sha256 is an
    /// error, because an unparseable sidecar and a missing one are the same
    /// operational fact.
    /// See docs/design/sidecar.md#sidecar-grammar.
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
                        "sidecar: fps must be a JSON string (\"30\", \"30000/1001\") \
                                so rational rates survive unchanged"
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
                // Unknown keys and informational strings are ignored, so a
                // preparation tool can add metadata without breaking deployed
                // players.
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

/// Where the frame rate came from: the asset's sidecar, or the test-rig-only
/// override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsSource {
    Sidecar,
    /// The `--test-rig-no-sidecar --fps <F>` override (test rig only).
    BenchOverride,
}

/// Decide the frame rate to play at, and where it came from.
///
/// | `--test-rig-no-sidecar` | sidecar fps | `--fps` | Result |
/// |---|---|---|---|
/// | no | present | absent | the sidecar's rate |
/// | no | present | same value | the sidecar's rate |
/// | no | present | different value | refused, naming both values |
/// | no | absent | either | refused |
/// | yes | either | a valid rate | the `--fps` value (test rig only) |
/// | yes | either | absent or invalid | refused |
///
/// With a sidecar present the sidecar decides; with none, startup refuses to
/// guess, and the two-flag test-rig override is the only way past. Comparison
/// with `--fps` is string equality, so `30` and `30/1` count as a mismatch.
/// See docs/design/sidecar.md#frame-rate-resolution and
/// docs/design/startup-checks.md#fail-closed-startup.
pub fn resolve_fps(
    sidecar_fps: Option<&str>,
    cli_fps: Option<&str>,
    test_rig_no_sidecar: bool,
) -> Result<(String, FpsSource), String> {
    if test_rig_no_sidecar {
        return match cli_fps {
            Some(f) if is_valid_fps(f) => Ok((f.to_string(), FpsSource::BenchOverride)),
            Some(f) => Err(format!("--fps {f:?} is not a valid frame rate")),
            None => Err("--test-rig-no-sidecar requires an explicit --fps".into()),
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
            "no sidecar found; refusing to guess the frame rate. Prepare the video \
             again with dex-sidecar write to produce <asset>.json, or use \
             --test-rig-no-sidecar --fps <F> (test rig only)"
                .into(),
        ),
    }
}

/// Check `payload`'s sha256 against the sidecar's, so a stale, wrong or
/// truncated asset is refused at startup instead of played.
/// See docs/design/sidecar.md#checksum-verification.
pub fn verify_payload(payload: &[u8], sidecar: &Sidecar) -> Result<(), String> {
    let actual = crate::sha256::sha256_hex(payload);
    if actual != sidecar.sha256 {
        return Err(format!(
            "asset does not match its sidecar: sha256 {actual} != sidecar {}; the asset or \
             sidecar is stale, wrong, or truncated — prepare the video again with \
             dex-sidecar write",
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
        assert!(e.contains("must be a JSON string"), "{e}");
    }

    #[test]
    fn missing_required_keys_are_refused_by_name() {
        let e = Sidecar::from_json(&format!(r#"{{"sha256":"{GOOD_SHA}"}}"#)).unwrap_err();
        assert!(e.contains("fps"), "{e}");
        let e = Sidecar::from_json(r#"{"fps":"30"}"#).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }

    #[test]
    fn a_malformed_sha256_is_refused_and_uppercase_is_stored_lowercase() {
        for sha in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            let text = format!(r#"{{"fps":"30","sha256":"{sha}"}}"#);
            assert!(Sidecar::from_json(&text).is_err(), "sha {sha:?} accepted");
        }
        let text = format!(r#"{{"fps":"30","sha256":"{}"}}"#, GOOD_SHA.to_uppercase());
        assert_eq!(Sidecar::from_json(&text).unwrap().sha256, GOOD_SHA);
    }

    #[test]
    fn the_fps_grammar_accepts_only_positive_integers_decimals_and_rationals() {
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
            "",                       // no object
            "[1,2]",                  // array at top level
            r#"{"a":{"b":1}}"#,       // nested object
            r#"{"a":[1]}"#,           // array value
            r#"{"a":true}"#,          // boolean
            r#"{"a":null}"#,          // null
            r#"{"a":-1}"#,            // negative number
            r#"{"a":1.5}"#,           // float
            r#"{"a":1e3}"#,           // exponent
            r#"{"a":"x"}"trailing"#,  // trailing data
            r#"{"a":"x""b":"y"}"#,    // missing comma
            r#"{"a":"unterminated}"#, // unterminated string
            r#"{"a":"x","a":"y"}"#,   // duplicate key
        ] {
            assert!(parse_flat_json(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn unicode_escapes_are_decoded() {
        // Each input is built with `format!`, so the JSON text the parser
        // receives holds a literal `\u` followed by four hex digits. The
        // expected values use Rust's own `\u{...}` syntax, which is unrelated
        // to the escape under test.

        // An escape for a code point that needs none.
        let json = format!(r#"{{"a":"\{}0041"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("A".to_string()));

        // What a serializer with escaping on by default writes for a
        // `source` filename that carries an accented character.
        let json = format!(r#"{{"a":"Z\{}00fcrich.mp4"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("Z\u{fc}rich.mp4".to_string()));

        // A code point too high for one escape, written as a UTF-16
        // surrogate pair (standard JSON).
        let json = format!(r#"{{"a":"\{0}d83c\{0}df89"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("\u{1F389}".to_string()));

        // A `\u` escape and a plain escape in one string.
        let json = format!(r#"{{"a":"caf\{}00e9\nmore"}}"#, 'u');
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str("caf\u{e9}\nmore".to_string()));
    }

    // A serializer that leaves the character alone (jq, serde_json, Python
    // with ensure_ascii=False) writes it as raw UTF-8 bytes. Both spellings
    // mean the same sidecar and both parse, so keep both accepted.
    // See docs/design/sidecar.md#sidecar-grammar.
    #[test]
    fn raw_utf8_in_an_informational_key_parses_like_its_escaped_form() {
        let raw = "Z\u{fc}rich.mp4".to_string();
        let json = format!(r#"{{"source":"{raw}"}}"#);
        let kv = parse_flat_json(&json).unwrap();
        assert_eq!(kv[0].1, Value::Str(raw.clone()));

        // The escaped spelling parses to the same value.
        let escaped = format!(r#"{{"source":"Z\{}00fcrich.mp4"}}"#, 'u');
        assert_eq!(parse_flat_json(&escaped).unwrap(), kv);

        // A full sidecar with such a source must still bind normally:
        // `source` is informational and never interpreted.
        let text = format!(r#"{{"fps":"30","sha256":"{GOOD_SHA}","source":"{raw}"}}"#);
        let s = Sidecar::from_json(&text).unwrap();
        assert_eq!(s.fps, "30");
    }

    // Duplicate rejection is the one grammar rule dexd states itself, in the
    // map visitor. Keep this test separate: a refactor that swapped the
    // visitor for a plain `serde_json::Map` would still pass every other test
    // in this file.
    // See docs/design/sidecar.md#sidecar-grammar.
    #[test]
    fn duplicate_keys_are_refused_and_named() {
        let e = parse_flat_json(r#"{"fps":"30","fps":"25"}"#).unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
        assert!(e.contains("fps"), "{e}");

        // The rule covers a duplicated key the sidecar never interprets: it
        // is about the document being unambiguous, whatever a key means.
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
            r#"{"a":"\ud800\udbff"}"#, // high surrogate followed by a second high surrogate
        ] {
            assert!(parse_flat_json(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn unicode_escape_in_an_ignored_key_does_not_block_startup() {
        // The whole path: a preparation tool's default-safe serializer
        // escapes a character inside `source`, a key dexd never interprets,
        // and the correctly hashed asset still binds. Built with `format!` so
        // the JSON text carries a literal escape.
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
    fn normal_startup_takes_the_fps_from_the_sidecar() {
        assert_eq!(
            resolve_fps(Some("30"), None, false).unwrap(),
            ("30".to_string(), FpsSource::Sidecar)
        );
    }

    #[test]
    fn an_fps_flag_must_agree_with_the_sidecar_and_a_mismatch_names_both_values() {
        assert!(resolve_fps(Some("30"), Some("30"), false).is_ok());
        let e = resolve_fps(Some("30"), Some("25"), false).unwrap_err();
        assert!(e.contains("30") && e.contains("25"), "{e}");
    }

    #[test]
    fn a_missing_sidecar_is_refused_without_the_test_rig_flag() {
        let e = resolve_fps(None, Some("30"), false).unwrap_err();
        assert!(e.contains("--test-rig-no-sidecar"), "{e}");
        assert!(resolve_fps(None, None, false).is_err());
    }

    #[test]
    fn the_test_rig_override_requires_both_flags_and_a_valid_rate() {
        assert_eq!(
            resolve_fps(None, Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        // With the override in force a present sidecar is ignored and the
        // --fps value wins.
        assert_eq!(
            resolve_fps(Some("25"), Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        assert!(resolve_fps(None, None, true).is_err());
        assert!(resolve_fps(None, Some("banana"), true).is_err());
    }

    #[test]
    fn an_asset_is_accepted_only_when_its_bytes_match_the_sidecar_checksum() {
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
