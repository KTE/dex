//! F6 — the exhibit display config: parse, validate, and decide the binding.
//!
//! Why: the display mode is a property of the INSTALLATION, not the asset.
//! One artwork may run on several panels over its life, and a panel may show
//! several artworks over a season — config whose lifetime differs from the
//! asset next to it eventually gets edited in the wrong copy. The
//! 2026-08-15 measurement is the concrete proof already on file: the same
//! `cmdline.txt` line is correct for one sink (Cam Link 4K, which vc4
//! refuses to build a 4K mode for unforced even though the sink's own EDID
//! prefers it) and wrong for another (Dell U2719DC, which must NOT carry the
//! force or it transmits a signal the panel cannot show). So the mode
//! belongs to an `/etc/dex/exhibit.{json,yaml}` conffile — venue truth, not
//! asset truth — parsed with the F3 sidecar's own hardened, fail-closed flat
//! subset grammar.
//!
//! CORRECTION (2026-08-17): an earlier draft of this doc comment claimed "the
//! M5 soak played a 2160p30 asset on a 1440p Dell" as field evidence that one
//! asset runs on several panels. That run never happened — the M5 soak ran
//! 4K30 on the Cam Link with the Dell disconnected. Removed rather than left
//! to mislead a future reader; the Cam-Link-vs-Dell force disagreement above
//! is real, bench-verified evidence and stands on its own.
//!
//! TWO FORMATS, AND THE EXTENSION DECIDES WHICH — see [`ConfigFormat`] for
//! why that dispatch is a correctness rule and not a convenience. `.json` is
//! strict JSON (machine-writable, and what the `.deb` ships); `.yaml` is YAML
//! (comments, no quoting ceremony — this file gets hand-edited in a venue,
//! possibly on a phone over SSH). The two are the same schema: only the ~40
//! lines that turn text into a flat key/value list differ, and
//! [`ExhibitConfig::from_pairs`] validates both.
//!
//! Format:
//!   {"display_mode":"3840x2160@30","kms_force":"3840x2160@30",
//!    "connector":"HDMI-A-1","display":"...","venue":"...","note":"..."}
//! or, identically:
//!   display_mode: 3840x2160@30    # what mpv is asked for
//!   kms_force: 3840x2160@30       # what the kernel cmdline must carry
//!   connector: HDMI-A-1
//! Required: `display_mode`. Optional: `kms_force` (default `"none"`),
//! `connector` (default `"HDMI-A-1"`), and the informational `display`,
//! `venue`, `note` strings, journal-logged at startup so the WHY that used to
//! live in a `config.txt` comment block travels with the config that is
//! actually enforced.
//!
//! UNKNOWN KEYS ARE REFUSED here — a deliberate divergence from the sidecar,
//! which tolerates them because ingest tooling evolves independently of
//! deployed players. This file has no such producer: it is hand-edited on the
//! same device the same `.deb` version reads it. Tolerating unknowns would
//! turn a typo (`"kms_forse"`) into a silently dropped force instead of a
//! caught one — a black gallery wall. Fail-closed IS the F-series semantics.
//!
//! Three further pure pieces live here, because none of them may live in
//! main.rs if they are to be testable without libmpv or real hardware
//! (§4 of the design):
//!
//! * [`resolve_display`] — mirrors `sidecar::resolve_fps`'s decision table:
//!   the config binds; an agreeing `--mode` cross-checks it; a disagreeing
//!   one is refused, naming both; an absent config (outside the bench escape
//!   hatch) is refused rather than guessed.
//! * [`check_cmdline_matches`] / [`cmdline_video_token`] — the gate that
//!   keeps the exhibit config and the KMS-layer `video=` token honest with
//!   each other, so editing one without the other is caught at the next
//!   start instead of black-screening a gallery for weeks.
//! * [`sysfs_modes_contains`] / [`mode_resolution`] — the pre-flight that
//!   catches "the configured resolution is not even in this connector's mode
//!   list" (the wrong-panel case) before asset/sidecar/NAL gates run.
//! * [`reconcile_cmdline`] — the pure rewrite `dex-exhibit-apply` (the
//!   privileged sibling binary) uses to keep `cmdline.txt` in sync,
//!   idempotently, preserving every other token untouched.

use crate::sidecar::{parse_flat_json, Value};
use yaml_rust2::{Yaml, YamlLoader};

/// Default path for the exhibit config the `.deb` SHIPS, as a dpkg conffile.
/// JSON rather than YAML because a conffile is a fixed path installed by a
/// machine: the shipped artifact should be the machine-writable format, and
/// an operator who prefers YAML replaces it (see
/// [`DEFAULT_EXHIBIT_CONFIG_PATHS`]) rather than editing what dpkg tracks.
pub const DEFAULT_EXHIBIT_CONFIG_PATH: &str = "/etc/dex/exhibit.json";

/// Where the player looks when `--exhibit-config` is not given, in order.
///
/// YAML first, so an operator who writes `exhibit.yaml` beside the shipped
/// `exhibit.json` gets what they wrote — the alternative (JSON wins, YAML
/// ignored) would let someone edit a file for an afternoon while the player
/// reads a different one, which is the exact "config drift" failure F6 exists
/// to end. `pick_default_config` refuses outright when both are present, so
/// "first wins" never silently decides anything: the order only fixes which
/// name the refusal calls the intended one.
pub const DEFAULT_EXHIBIT_CONFIG_PATHS: [&str; 2] =
    ["/etc/dex/exhibit.yaml", "/etc/dex/exhibit.json"];

/// Choose the default config among those that actually exist on disk.
///
/// `existing` is the subset of [`DEFAULT_EXHIBIT_CONFIG_PATHS`] that exists,
/// in that array's order; main.rs does the `Path::exists` calls so this stays
/// pure and testable without a filesystem.
///
/// * exactly one → that one
/// * none → `None`, and the caller states the "no exhibit config" refusal
///   (one message, one place — `resolve_display`'s)
/// * both → **refuse**. Two configs for one player is the same class of fact
///   as a `--mode` that contradicts the config: there is a real, answerable
///   question about which the operator meant, and answering it by precedence
///   would hide it. Deleting the loser is one command; debugging a venue
///   running yesterday's mode is a day.
pub fn pick_default_config<'a>(existing: &[&'a str]) -> Result<Option<&'a str>, String> {
    match existing {
        [] => Ok(None),
        [only] => Ok(Some(only)),
        several => {
            // Name the LIKELY cause, not just the rule. The overwhelmingly
            // common way to reach this is switching to YAML: writing
            // exhibit.yaml leaves the .deb's own exhibit.json conffile sitting
            // beside it, so the operator did one correct thing and got a
            // refusal. A message that only restates the invariant would make
            // that look like a bug in the player.
            let shipped: Vec<&str> = several
                .iter()
                .copied()
                .filter(|p| p.ends_with(".json"))
                .collect();
            let hint = if shipped.is_empty() {
                String::new()
            } else {
                format!(
                    " If you have just switched to YAML, the leftover is the one the package \
                     installs: sudo rm {}.",
                    shipped.join(" ")
                )
            };
            Err(format!(
                "{} exhibit configs exist at once ({}) and nothing here can know which one you \
                 meant. Keep exactly one — delete or rename the others — or name the intended \
                 file explicitly with --exhibit-config.{hint}",
                several.len(),
                several.join(", ")
            ))
        }
    }
}
pub const DEFAULT_CONNECTOR: &str = "HDMI-A-1";
pub const DEFAULT_KMS_FORCE: &str = "none";

/// Is `t` a non-empty run of ASCII digits with at least one nonzero digit —
/// the same "positive integer" shape `sidecar::is_valid_fps` uses for its
/// numerator/denominator, duplicated here (rather than exposed from
/// `sidecar`) because it is a two-line primitive, not a shared contract.
fn positive_int(t: &str) -> bool {
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && t.bytes().any(|b| b != b'0')
}

/// Is `t` a non-empty run of ASCII digits (leading zeros and an all-zero
/// value both allowed — connector numbering is not a rate, "0" is a
/// legitimate enumeration index on some drivers).
fn digits_only(t: &str) -> bool {
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())
}

/// Split `"WxH@R"` into its three fields. Neither half is validated here —
/// callers apply their own grammar to `w`/`h`/`r`.
fn split_mode(s: &str) -> Option<(&str, &str, &str)> {
    let (wh, r) = s.split_once('@')?;
    let (w, h) = wh.split_once('x')?;
    Some((w, h, r))
}

/// `display_mode` grammar: `"auto"`, or `"WxH@R"` with W, H, R positive
/// INTEGERS — the same refresh rule as `kms_force`, and deliberately NOT the
/// `--fps` grammar an earlier revision borrowed. That revision reasoned
/// "mpv's `--drm-mode` accepts a fractional refresh, so `@29.97` and
/// `@30000/1001` are representable"; bench-driving both through the real
/// deploy path (dexpi4, mpv 0.40, 2026-08-17) disproved it twice over:
///
/// * `@30000/1001` fails mpv's OPTION PARSER outright (`set
///   drm-mode=3840x2160@30000/1001: error setting option (-7)`) — a config
///   value this grammar accepted could NEVER play, only produce a 2 s-cadence
///   restart loop. Fail-closed belongs at config parse, not at VO init.
/// * `@29.97` parses and PLAYS — because mpv matches DRM modes by integer
///   `vrefresh` rounding, i.e. it silently drove the same 30 Hz mode that
///   `@30` names honestly. A decimal buys nothing over its rounded integer
///   (the kernel mode's timing is what it is) while implying a precision
///   that does not exist — the silent-wrongness class this crate refuses.
///
/// (An INTEGER refresh the connector does not offer is caught loudly at VO
/// init — `Could not find mode matching 3840x2160@60`, same bench — since
/// the sysfs pre-flight can only validate the resolution half; see
/// `mode_resolution`.) The `@R` part is mandatory: `"3840x2160"` alone would
/// let mpv pick among same-resolution timings by list order, which is again
/// silent wrongness.
pub fn is_valid_display_mode(s: &str) -> bool {
    if s == "auto" {
        return true;
    }
    match split_mode(s) {
        Some((w, h, r)) => positive_int(w) && positive_int(h) && positive_int(r),
        None => false,
    }
}

/// `kms_force` grammar: `"none"`, or `"WxH@R"`/`"WxH@RD"` with W, H, R
/// positive integers — R is an integer here, unlike `display_mode`, because
/// the kernel's `video=` cmdline grammar has no fractional refresh. The
/// trailing `D` forces the connector to read `connected` even when nothing is
/// attached yet (the boot-order insurance F6 replaces the old comment block
/// with); it is a suffix on the whole token, not part of the refresh number.
pub fn is_valid_kms_force(s: &str) -> bool {
    if s == "none" {
        return true;
    }
    let core = s.strip_suffix('D').unwrap_or(s);
    match split_mode(core) {
        Some((w, h, r)) => positive_int(w) && positive_int(h) && positive_int(r),
        None => false,
    }
}

/// `asset` grammar: an ABSOLUTE path to a file, with no trailing slash and no
/// ASCII control characters.
///
/// Absolute because the player runs as a systemd service whose working
/// directory is `/`: a relative path would resolve somewhere the operator did
/// not type, and — being a *plausible* path — would fail with "no such file"
/// pointing at a name that looks right. Control characters because this string
/// is printed into the journal at every start, and the journal is the only
/// diagnostic channel a gallery device has; a newline inside it would forge a
/// second log line.
///
/// Deliberately NOT checked here: whether the file exists, or what extension
/// it has. Existence is main.rs's job (it produces the read error, which is
/// more informative than a grammar refusal), and this crate has no business
/// deciding that an artwork must be called `.265`.
pub fn is_valid_asset(s: &str) -> bool {
    s.starts_with('/')
        && !s.ends_with('/')
        && !s.chars().any(|c| c.is_ascii_control())
}

/// `connector` grammar: `"HDMI-A-<n>"`, n a plain digit run. Also governs the
/// `video=<connector>:...` token dex-exhibit-apply writes and the
/// `/sys/class/drm/card*-<connector>` glob the sysfs pre-flight reads, so
/// accepting garbage here would surface far from where it was typed.
pub fn is_valid_connector(s: &str) -> bool {
    match s.strip_prefix("HDMI-A-") {
        Some(n) => digits_only(n),
        None => false,
    }
}

/// Which parser reads an exhibit config — decided by the FILE EXTENSION, never
/// by sniffing the bytes.
///
/// YAML is a superset of JSON, so "parse everything as YAML" would pass every
/// test this file could write and still be wrong: it would accept comments,
/// anchors and unquoted keys inside a file named `.json`, and that file would
/// then break `jq`, `python -m json.tool`, and any other consumer that trusts
/// the name. **An extension is a promise to the rest of the world about what
/// the bytes are.** Honouring it is the entire reason for offering two formats
/// instead of one, so the dispatch is here, at the outermost layer, where it
/// cannot be bypassed by a convenience helper.
///
/// **Why not YAML's own JSON schema, which exists for exactly this?** Because
/// it solves a different problem than the one here. YAML 1.2 defines three
/// schemas (failsafe, JSON, core), and all three govern only how an *untagged
/// scalar* resolves to a type — they say nothing about SYNTAX. A YAML parser
/// set to the JSON schema still accepts `#` comments, block style, unquoted
/// keys, anchors and `---` document markers; it would simply resolve
/// `3840x2160` differently. So "parse `.json` with a YAML parser in JSON-schema
/// mode" would NOT deliver the promise a `.json` name makes, which is precisely
/// the promise this type exists to keep. That is why the JSON path runs on
/// `serde_json`, a real JSON parser, rather than on a configured YAML one.
///
/// It would also be the wrong choice for the `.yaml` path, in the opposite
/// direction: under the JSON schema a plain scalar matching none of
/// null/bool/int/float is an ERROR, so `display_mode: 3840x2160@30` — an
/// unquoted string, and the entire ergonomic point of offering YAML — would
/// fail to resolve. (Moot in practice: `yaml-rust2` hardwires core-ish
/// resolution and exposes no schema selection at all. See
/// [`parse_flat_yaml`], which documents the one place it departs from 1.2
/// core.)
///
/// Everything after tree-building is shared: both parsers produce the same
/// `Vec<(String, Value)>` flat map that [`crate::sidecar::parse_flat_json`]
/// already produces, and [`ExhibitConfig::from_pairs`] does 100% of the
/// mapping and validation for both. Only the ~40 lines that turn text into
/// pairs differ. The subset that flat map enforces — strings and non-negative
/// integers, one level deep — is stricter in node types than any of YAML's
/// three schemas, but it applies AFTER resolution, so resolution decisions
/// remain observable: `venue: 2026` resolves to an integer and is then refused
/// as "must be a string", exactly as `"venue": 2026` is on the JSON side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Yaml,
}

impl ConfigFormat {
    /// Decide the format from a path's extension, case-insensitively (an
    /// operator's editor may have written `EXHIBIT.YAML`, and refusing that
    /// would be pedantry rather than safety — the promise the extension makes
    /// is the same either way).
    ///
    /// An unrecognised or absent extension REFUSES rather than defaulting to
    /// either parser. Defaulting is how a `.txt` full of YAML ends up being
    /// read as JSON, or worse, the reverse: silently accepting YAML in a file
    /// the rest of the toolchain will read as JSON is precisely the failure
    /// this type exists to make impossible.
    ///
    /// Uses `Path::extension` rather than splitting on the last `.`, so a
    /// DOTFILE named `.json` has no extension and refuses — which is what the
    /// rest of the world thinks too, and the point of this function is to
    /// agree with the rest of the world about what a file name means.
    pub fn from_path(path: &str) -> Result<ConfigFormat, String> {
        let ext = std::path::Path::new(path)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        match ext.as_deref() {
            Some("json") => Ok(ConfigFormat::Json),
            Some("yaml") | Some("yml") => Ok(ConfigFormat::Yaml),
            _ => Err(format!(
                "exhibit config {path:?}: cannot tell the format from the file name — name it \
                 .json (strict JSON) or .yaml/.yml (YAML). The extension decides the parser, so \
                 that whatever else reads this file gets the format its name promises"
            )),
        }
    }
}

/// Parse `text` as a single flat YAML mapping under the SAME subset grammar
/// [`crate::sidecar::parse_flat_json`] enforces for JSON — strings and
/// non-negative integers only, one level deep, no duplicate keys — returning
/// its key/value pairs in source order.
///
/// The subset is not a limitation grudgingly inherited from the JSON side; it
/// is what keeps the two formats interchangeable. A YAML feature this refuses
/// (a nested mapping, a list, an anchor) is one that could not be written in
/// the `.json` form of the same config, and a config whose meaning depends on
/// which extension it was saved under would defeat the point of supporting
/// both.
///
/// Three YAML-specific hazards, all refused rather than coerced:
///
/// * **Booleans.** `display_mode: true` is a boolean, not the string
///   `"true"`, and refuses with a message naming the quoting fix.
///
///   MEASURED, not assumed, because the received wisdom here is wrong for
///   this library: `yaml-rust2` 0.11 resolves scalars close to the **YAML 1.2
///   core schema**, where ONLY `true`/`false` (any of three casings) are
///   booleans. The famous "Norway problem" — `no` silently becoming `false` —
///   is a YAML **1.1** behaviour and does not occur here, so `kms_force: no`
///   arrives as the string `"no"` and is refused a step later by
///   [`is_valid_kms_force`]'s grammar, naming the valid values. Both paths
///   refuse; only the message differs. Pinned by test in both directions, so
///   a version bump that adopted 1.1 resolution could not slip through.
///
///   "Close to", not "is": its null resolution (`yaml.rs`'s `from_str`) is
///   `"" | "~" | "null"`, where the 1.2 core schema also lists `Null` and
///   `NULL`. Driven against the real binary — `note: Null` and `note: NULL`
///   are accepted as STRINGS, while `null`, `~` and an empty value resolve to
///   null and are refused. Harmless for this schema (only the informational
///   keys could receive such a value, and taking it literally is the friendlier
///   of the two readings) but stated exactly, because "it implements the core
///   schema" is the kind of nearly-true sentence a future reader would rely on.
///
///   **There is no schema knob to reach for**: `yaml-rust2` hardwires this
///   resolution and exposes no way to select the failsafe or JSON schema. See
///   [`ConfigFormat`] for why the JSON schema would not have been the right
///   tool for the `.json` path anyway.
/// * **Floats.** `29.97` is `Yaml::Real`. The JSON side already refuses
///   non-integers, and the bench evidence in [`is_valid_display_mode`] is why:
///   a decimal refresh silently means its rounded integer.
/// * **Multiple documents.** A `---`-separated stream has no single answer to
///   "what is the config", so it refuses instead of taking the first.
///
/// Duplicate keys are rejected by `yaml-rust2` itself (its loader errors on
/// insert rather than last-wins, unlike most YAML libraries) — the one rule
/// the JSON side had to hand-roll a serde visitor for. Pinned by test, not
/// assumed: see `yaml_duplicate_key_refused`.
pub fn parse_flat_yaml(text: &str) -> Result<Vec<(String, Value)>, String> {
    let docs = YamlLoader::load_from_str(text).map_err(|e| format!("exhibit config YAML: {e}"))?;
    let doc = match docs.len() {
        // load_from_str returns zero documents for empty or comment-only
        // input. Treated as a parse failure, not an empty config: a config
        // that parses to "no keys at all" would then fail on the missing
        // required key with a message about `display_mode`, burying the real
        // problem (the file is blank -- truncated write, wrong path, editor
        // that saved nothing).
        0 => {
            return Err(
                "exhibit config YAML: no document — the file is empty or contains only comments"
                    .into(),
            )
        }
        1 => &docs[0],
        n => {
            return Err(format!(
                "exhibit config YAML: {n} documents in one file (`---` separators) — an exhibit \
                 config must be exactly one mapping, since nothing here could say which document \
                 is the authoritative one"
            ))
        }
    };
    let map = match doc {
        Yaml::Hash(h) => h,
        other => {
            return Err(format!(
                "exhibit config YAML: top level is {}, expected a mapping of key: value pairs",
                yaml_type_name(other)
            ))
        }
    };
    let mut out = Vec::with_capacity(map.len());
    for (k, v) in map {
        let key = match k {
            Yaml::String(s) => s.clone(),
            other => {
                return Err(format!(
                    "exhibit config YAML: key is {}, expected a string",
                    yaml_type_name(other)
                ))
            }
        };
        let value = match v {
            Yaml::String(s) => Value::Str(s.clone()),
            // Non-negative only, matching the JSON subset's u64. A negative
            // number is not merely out of range for these keys, it is out of
            // range for the FORMAT -- so it refuses here, in the same voice a
            // `.json` file's `-1` would.
            Yaml::Integer(i) if *i >= 0 => Value::Num(*i as u64),
            Yaml::Integer(i) => {
                return Err(format!(
                    "exhibit config YAML: key {key:?} has negative value {i} — this format \
                     carries strings and non-negative integers only"
                ))
            }
            Yaml::Boolean(_) => {
                return Err(format!(
                    "exhibit config YAML: key {key:?} resolved to a BOOLEAN — YAML reads bare \
                     true/false as booleans, not as the text \"true\"/\"false\". Quote the value \
                     if you meant a string"
                ))
            }
            other => {
                return Err(format!(
                    "exhibit config YAML: key {key:?} has a {} value — this format carries \
                     strings and non-negative integers only, one level deep",
                    yaml_type_name(other)
                ))
            }
        };
        out.push((key, value));
    }
    Ok(out)
}

/// Name a `Yaml` node's kind for an error message, in the vocabulary an
/// operator editing YAML would recognise — not the Rust variant name.
fn yaml_type_name(y: &Yaml) -> &'static str {
    match y {
        Yaml::Real(_) => "a decimal number (this format takes integers only)",
        Yaml::Integer(_) => "an integer",
        Yaml::String(_) => "a string",
        Yaml::Boolean(_) => "a boolean",
        Yaml::Array(_) => "a list",
        Yaml::Hash(_) => "a nested mapping",
        Yaml::Alias(_) => "an alias (`*anchor`)",
        Yaml::Null => "null (an empty value)",
        Yaml::BadValue => "unreadable",
    }
}

/// Does `p` exist, distinguishing "not there" from "cannot tell"?
///
/// `Path::exists()` would be one line, but it maps EVERY error to `false` —
/// including EACCES on a parent directory. That is the same misreport the F6
/// review already caught once on the read path (an unreadable config
/// producing "create this file" for a file the operator can see exists), and
/// the convenient call would quietly reintroduce it.
fn config_exists(p: &str) -> Result<bool, String> {
    match std::fs::metadata(p) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "cannot stat {p}: {e} -- this is not \"missing\", it is \"cannot tell\"; check the \
             permissions on {p} and on every directory above it (the dex user must be able to \
             traverse them)"
        )),
    }
}

/// Locate, read and parse the exhibit config, returning it with the path it
/// actually came from. `Ok(None)` means no config exists.
///
/// **The one function in this module that touches the filesystem**, and it
/// earns the exception: WHICH FILE IS THE CONFIG is a policy, and both
/// binaries that answer it — `dex-loop`, which enforces the config, and
/// `dex-exhibit-apply`, which reconciles the boot cmdline *to* the config —
/// must answer it identically. Two copies of this logic would drift, and the
/// specific way they would drift is that the apply tool writes a cmdline for
/// one file while the player reads another: F6's own failure mode, produced by
/// F6's own implementation. (The module's testability rule is about libmpv and
/// real DRM, not about `stat` — `defaults` is injectable precisely so this is
/// testable against a temp directory.)
///
/// `Ok(None)` is deliberately NOT an error: [`resolve_display`] is the single
/// place that states the "no exhibit config" refusal, mirroring how
/// `sidecar::resolve_fps` states the analogous "no sidecar" one. Every OTHER
/// failure is stated here, because each is a distinct operational fact with a
/// distinct repair.
pub fn load_exhibit_config(
    override_path: Option<&str>,
    defaults: &[&str],
) -> Result<Option<(ExhibitConfig, String)>, String> {
    let path = match override_path {
        // An EXPLICITLY NAMED file that is not there is not the same fact as
        // "no config was ever installed", and must not borrow the latter's
        // message: "create /etc/dex/exhibit.json" is actively wrong advice for
        // an operator who just told us to read something else.
        Some(p) => {
            if !config_exists(p)? {
                return Err(format!(
                    "--exhibit-config {p:?}: no such file. (This is the explicitly named path; \
                     drop --exhibit-config to use the installed default.)"
                ));
            }
            p.to_string()
        }
        None => {
            let mut existing = Vec::new();
            for p in defaults {
                if config_exists(p)? {
                    existing.push(*p);
                }
            }
            match pick_default_config(&existing)? {
                Some(p) => p.to_string(),
                None => return Ok(None),
            }
        }
    };
    let format = ConfigFormat::from_path(&path)?;
    // Reaching this read means the file existed a moment ago, so NotFound is
    // no longer the expected miss -- every error here is a real one.
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "cannot read {path}: {e} -- the file exists but is not readable; check its \
             owner/permissions (the dex user must be able to read it)"
        )
    })?;
    let cfg = ExhibitConfig::parse(&text, format).map_err(|e| format!("{path}: {e}"))?;
    Ok(Some((cfg, path)))
}

/// A parsed, validated exhibit config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExhibitConfig {
    /// WHICH artwork this exhibit plays. Optional in the file, but something
    /// must supply it -- see [`resolve_asset`].
    pub asset: Option<String>,
    pub display_mode: String,
    pub kms_force: String,
    pub connector: String,
    pub display: Option<String>,
    pub venue: Option<String>,
    pub note: Option<String>,
}

impl ExhibitConfig {
    /// Parse and validate an exhibit config, with `format` deciding the
    /// parser. Fail-closed, same theory as the F3 sidecar: an unparseable
    /// exhibit config and a missing one are the same operational fact — both
    /// refuse rather than run on a display nobody stated.
    ///
    /// Callers get `format` from [`ConfigFormat::from_path`], never from the
    /// bytes: see that type's docs for why sniffing would be wrong even though
    /// it would always work.
    pub fn parse(text: &str, format: ConfigFormat) -> Result<ExhibitConfig, String> {
        match format {
            ConfigFormat::Json => Self::from_json(text),
            ConfigFormat::Yaml => Self::from_yaml(text),
        }
    }

    /// Parse and validate an exhibit config's **strict JSON** text.
    pub fn from_json(text: &str) -> Result<ExhibitConfig, String> {
        Self::from_pairs(parse_flat_json(text).map_err(|e| format!("exhibit config JSON: {e}"))?)
    }

    /// Parse and validate an exhibit config's **YAML** text.
    pub fn from_yaml(text: &str) -> Result<ExhibitConfig, String> {
        Self::from_pairs(parse_flat_yaml(text)?)
    }

    /// Map a parsed flat key/value tree onto the struct, and validate it.
    ///
    /// **This is the whole schema, and both formats reach it unchanged.** The
    /// two parsers above differ only in how text becomes `kv`; every key name,
    /// default, grammar check and error message lives here exactly once, so a
    /// `.json` and a `.yaml` file expressing the same config cannot diverge in
    /// meaning or in what they refuse.
    ///
    /// UNLIKE the sidecar, unknown keys are refused (module docs above) —
    /// this is the one place this parser's contract deliberately differs
    /// from `Sidecar::from_json`'s.
    fn from_pairs(kv: Vec<(String, Value)>) -> Result<ExhibitConfig, String> {
        let mut asset = None;
        let mut display_mode = None;
        let mut kms_force = None;
        let mut connector = None;
        let mut display = None;
        let mut venue = None;
        let mut note = None;
        for (k, v) in kv {
            match (k.as_str(), v) {
                ("asset", Value::Str(s)) => asset = Some(s),
                ("asset", Value::Num(_)) => {
                    return Err("exhibit config: asset must be a string".into())
                }
                ("display_mode", Value::Str(s)) => display_mode = Some(s),
                ("display_mode", Value::Num(_)) => {
                    return Err("exhibit config: display_mode must be a string".into())
                }
                ("kms_force", Value::Str(s)) => kms_force = Some(s),
                ("kms_force", Value::Num(_)) => {
                    return Err("exhibit config: kms_force must be a string".into())
                }
                ("connector", Value::Str(s)) => connector = Some(s),
                ("connector", Value::Num(_)) => {
                    return Err("exhibit config: connector must be a string".into())
                }
                ("display", Value::Str(s)) => display = Some(s),
                ("display", Value::Num(_)) => {
                    return Err("exhibit config: display must be a string".into())
                }
                ("venue", Value::Str(s)) => venue = Some(s),
                ("venue", Value::Num(_)) => {
                    return Err("exhibit config: venue must be a string".into())
                }
                ("note", Value::Str(s)) => note = Some(s),
                ("note", Value::Num(_)) => {
                    return Err("exhibit config: note must be a string".into())
                }
                (other, _) => {
                    return Err(format!(
                        "exhibit config: unknown key {other:?} (strict schema, unlike the \
                         sidecar — known keys: asset, display_mode, kms_force, connector, \
                         display, venue, note; a typo here must not silently drop a force)"
                    ))
                }
            }
        }
        let display_mode =
            display_mode.ok_or("exhibit config: missing required key \"display_mode\"")?;
        if !is_valid_display_mode(&display_mode) {
            return Err(format!(
                "exhibit config: invalid display_mode {display_mode:?} (expect \"auto\" or \
                 \"WxH@R\", e.g. \"3840x2160@30\")"
            ));
        }
        let kms_force = kms_force.unwrap_or_else(|| DEFAULT_KMS_FORCE.to_string());
        if !is_valid_kms_force(&kms_force) {
            return Err(format!(
                "exhibit config: invalid kms_force {kms_force:?} (expect \"none\" or \
                 \"WxH@R\"/\"WxH@RD\" with an integer refresh)"
            ));
        }
        let connector = connector.unwrap_or_else(|| DEFAULT_CONNECTOR.to_string());
        if !is_valid_connector(&connector) {
            return Err(format!(
                "exhibit config: invalid connector {connector:?} (expect \"HDMI-A-<n>\")"
            ));
        }
        if let Some(a) = &asset {
            if !is_valid_asset(a) {
                return Err(format!(
                    "exhibit config: invalid asset {a:?} (expect an absolute path to the file \
                     to play, e.g. \"/opt/dex/loop.265\" -- no trailing slash, no control \
                     characters)"
                ));
            }
        }
        Ok(ExhibitConfig {
            asset,
            display_mode,
            kms_force,
            connector,
            display,
            venue,
            note,
        })
    }
}

/// Where a bound display config came from: the exhibit config, or the bench
/// escape hatch (`--bench-no-sidecar` [`--mode <M>`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySource {
    Config,
    Bench,
}

/// The resolved display binding: what the player asks mpv for, what the
/// kernel cmdline is expected to carry, and which connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDisplay {
    pub display_mode: String,
    pub kms_force: String,
    pub connector: String,
    pub source: DisplaySource,
}

/// Decide the display binding, mirroring `sidecar::resolve_fps`'s table.
///
/// | config | `--mode` | bench | result |
/// |---|---|---|---|
/// | present | absent | no | config binds |
/// | present | == config | no | config binds (cross-check) |
/// | present | != config | no | refuse, naming both |
/// | absent | any | no | refuse: no exhibit config |
/// | any | any | yes | CLI wins (`--mode` or `"auto"`); config ignored |
///
/// Under the bench flag the cmdline gate is ALSO skipped (main.rs), since a
/// `~/bench` build runs on hand-managed boot state by definition — that is
/// why this function reports `kms_force: "none"` and the default connector
/// for the bench branch rather than anything derived from a config that is,
/// by definition, not consulted.
pub fn resolve_display(
    config: Option<&ExhibitConfig>,
    cli_mode: Option<&str>,
    bench_no_sidecar: bool,
) -> Result<ResolvedDisplay, String> {
    if bench_no_sidecar {
        return match cli_mode {
            Some(m) if is_valid_display_mode(m) => Ok(ResolvedDisplay {
                display_mode: m.to_string(),
                kms_force: DEFAULT_KMS_FORCE.to_string(),
                connector: DEFAULT_CONNECTOR.to_string(),
                source: DisplaySource::Bench,
            }),
            Some(m) => Err(format!(
                "--mode {m:?} is not a valid display mode (expect \"auto\" or \"WxH@R\", e.g. \
                 \"3840x2160@30\")"
            )),
            None => Ok(ResolvedDisplay {
                display_mode: "auto".to_string(),
                kms_force: DEFAULT_KMS_FORCE.to_string(),
                connector: DEFAULT_CONNECTOR.to_string(),
                source: DisplaySource::Bench,
            }),
        };
    }
    match config {
        None => Err(
            "no exhibit config; refusing to guess the display. Create /etc/dex/exhibit.json \
             (the .deb ships one) or use --bench-no-sidecar on a bench"
                .into(),
        ),
        Some(cfg) => match cli_mode {
            None => Ok(ResolvedDisplay {
                display_mode: cfg.display_mode.clone(),
                kms_force: cfg.kms_force.clone(),
                connector: cfg.connector.clone(),
                source: DisplaySource::Config,
            }),
            Some(m) if m == cfg.display_mode => Ok(ResolvedDisplay {
                display_mode: cfg.display_mode.clone(),
                kms_force: cfg.kms_force.clone(),
                connector: cfg.connector.clone(),
                source: DisplaySource::Config,
            }),
            Some(m) => Err(format!(
                "--mode {m:?} contradicts exhibit config display_mode {:?}; drop --mode (the \
                 exhibit config is authoritative) or fix /etc/dex/exhibit.json",
                cfg.display_mode
            )),
        },
    }
}

/// Where the asset path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSource {
    /// The exhibit config's `asset` key — the deployment path.
    Config,
    /// A path given on the command line. Legitimate on a bench, and as a
    /// one-off on a device without editing `/etc`; never how a show runs.
    Cli,
}

/// The resolved asset: which file to play, and who said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub path: String,
    pub source: AssetSource,
}

/// Decide WHICH ASSET to play — the capability the exhibit file exists for.
///
/// Before this, `dex-loop.service`'s `ExecStart` hardcoded
/// `/opt/dex/loop.265`, so changing the artwork meant overwriting that one
/// path or editing a systemd unit the package owns. Several assets could not
/// sit in storage with the exhibit choosing one, which was Max's stated reason
/// for wanting an exhibit file at all: an exhibit is a pairing of a venue with
/// an artwork, and it could previously express only the venue half.
///
/// The table mirrors [`resolve_display`] and `sidecar::resolve_fps` exactly,
/// because a third decision surface with its own idiom is how invariants get
/// forgotten:
///
/// | config `asset` | CLI path | bench | result |
/// |---|---|---|---|
/// | present | absent | no | config binds |
/// | present | == config | no | config binds (cross-check) |
/// | present | != config | no | refuse, naming both |
/// | absent | present | no | CLI binds (one-off, logged as such) |
/// | absent | absent | no | refuse: nothing names an asset |
/// | any | present | yes | CLI binds; config ignored |
/// | any | absent | yes | refuse: bench must be explicit |
///
/// The row that could have gone the other way is the fifth. Defaulting to
/// `/opt/dex/loop.265` would carry every existing deployment through untouched
/// — and would be the worst guess in the program: it would silently play LAST
/// season's artwork for an operator who edited the config but mistyped the
/// key, with every metric green. Every other guess this crate refuses (frame
/// rate, display mode) is refused for a weaker version of that reason. So:
/// refuse, name the exact line to add, and warn at package-install time (see
/// `deploy/maintainer-scripts/postinst`) so an upgrade surfaces it while
/// someone is watching rather than at the next power cycle.
pub fn resolve_asset(
    config: Option<&ExhibitConfig>,
    cli_path: Option<&str>,
    bench_no_sidecar: bool,
) -> Result<ResolvedAsset, String> {
    if bench_no_sidecar {
        return match cli_path {
            Some(p) => Ok(ResolvedAsset {
                path: p.to_string(),
                source: AssetSource::Cli,
            }),
            None => Err(
                "--bench-no-sidecar consults no exhibit config, so the asset must be given on \
                 the command line: dex-loop <stream.265> --bench-no-sidecar --fps <F>"
                    .into(),
            ),
        };
    }
    match (config.and_then(|c| c.asset.as_deref()), cli_path) {
        (Some(a), None) => Ok(ResolvedAsset {
            path: a.to_string(),
            source: AssetSource::Config,
        }),
        (Some(a), Some(p)) if a == p => Ok(ResolvedAsset {
            path: a.to_string(),
            source: AssetSource::Config,
        }),
        (Some(a), Some(p)) => Err(format!(
            "command-line asset {p:?} contradicts the exhibit config's asset {a:?}; drop the \
             path (the exhibit config is authoritative) or fix the config"
        )),
        (None, Some(p)) => Ok(ResolvedAsset {
            path: p.to_string(),
            source: AssetSource::Cli,
        }),
        (None, None) => Err(
            "no asset: the exhibit config does not name one and none was given on the command \
             line. Add it to the exhibit config -- `asset: /opt/dex/loop.265` (YAML) or \
             `\"asset\": \"/opt/dex/loop.265\"` (JSON) -- which is what lets several assets sit \
             in /opt/dex with the exhibit choosing one"
                .into(),
        ),
    }
}

/// Find the `video=<connector>:<mode>` token for exactly `connector` inside a
/// kernel cmdline string (space-separated tokens, as `/proc/cmdline` and
/// `cmdline.txt` both are). Other connectors' `video=` tokens, and every
/// other kind of token, are ignored. Returns the `<mode>` half only.
pub fn cmdline_video_token(cmdline: &str, connector: &str) -> Option<String> {
    cmdline.split_whitespace().find_map(|tok| {
        let rest = tok.strip_prefix("video=")?;
        let (conn, mode) = rest.split_once(':')?;
        (conn == connector).then(|| mode.to_string())
    })
}

/// Find a CONNECTORLESS `video=` token (`video=WxH@R`, no `conn:` prefix) —
/// the kernel's grammar also accepts this shape, and it forces ALL
/// connectors. Neither this module's gate nor its reconciler can reason
/// about one (which connector does it bind? how does the kernel arbitrate it
/// against a per-connector token?), so callers REFUSE when one is present
/// rather than reporting "no token" (the gate) or appending a second,
/// overlapping force (the reconciler) — refusing beats guessing at kernel
/// arbitration order for a token shape no dex install is supposed to carry.
/// Returns the whole token, for the refusal message to name verbatim.
pub fn connectorless_video_token(cmdline: &str) -> Option<String> {
    cmdline.split_whitespace().find_map(|tok| {
        let rest = tok.strip_prefix("video=")?;
        (!rest.contains(':')).then(|| tok.to_string())
    })
}

/// The gate that keeps the exhibit config and the KMS-layer `video=` token
/// honest with each other (§3.3 of the design): an operator who edits one and
/// forgets the other must be refused, loudly, naming what to run — not
/// black-screen the gallery on whichever value the kernel happens to have
/// booted with. `kms_force == "none"` means "expect NO token for this
/// connector"; anything else means "expect this EXACT token".
/// Every refusal names BOTH possible repairs, because this gate cannot know
/// which side is stale: the config may be right and the cmdline leftover —
/// but equally the cmdline may carry a force this venue deliberately needs
/// (the Cam Link builds no 4K mode unforced) that a freshly installed
/// default config simply does not know about yet. A message prescribing only
/// `dex-exhibit-apply` in that second case would instruct the operator to
/// DELETE the needed force, after which `display_mode=auto` silently plays
/// at whatever the connector negotiates — the exact wrongness class this
/// gate exists to close, reached by following its own instructions.
pub fn check_cmdline_matches(cmdline: &str, connector: &str, kms_force: &str) -> Result<(), String> {
    if let Some(tok) = connectorless_video_token(cmdline) {
        return Err(format!(
            "the kernel cmdline carries a connectorless token {tok:?}, which forces ALL \
             connectors -- this gate cannot reconcile it with the exhibit config's \
             per-connector kms_force. Qualify it with a connector \
             (video={connector}:<mode>) or remove it from /boot/firmware/cmdline.txt, \
             then reboot"
        ));
    }
    let found = cmdline_video_token(cmdline, connector);
    match (kms_force, found) {
        (DEFAULT_KMS_FORCE, None) => Ok(()),
        (DEFAULT_KMS_FORCE, Some(found)) => Err(format!(
            "exhibit config says kms_force=none for {connector}, but the kernel cmdline \
             carries video={connector}:{found} -- EITHER the config is stale (a force this \
             venue deliberately needs, e.g. a display that builds no 4K mode unforced: set \
             kms_force={found:?} and a matching display_mode in /etc/dex/exhibit.json to \
             keep it) OR the cmdline is (run 'sudo dex-exhibit-apply' and reboot to remove \
             the force)"
        )),
        (want, Some(found)) if want == found => Ok(()),
        (want, Some(found)) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries \
             video={connector}:{found} -- EITHER the config is stale (fix kms_force in \
             /etc/dex/exhibit.json to match the venue) OR the cmdline is (run 'sudo \
             dex-exhibit-apply' and reboot)"
        )),
        (want, None) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries no video= \
             token for {connector} -- EITHER the config is stale (set kms_force=none in \
             /etc/dex/exhibit.json if this venue needs no force) OR the cmdline is (run \
             'sudo dex-exhibit-apply' and reboot to add the token)"
        )),
    }
}

/// The `WxH` half of a `display_mode` (`"auto"` has none — sysfs has no
/// "auto" entry to check against, so callers must skip the pre-flight for it).
pub fn mode_resolution(display_mode: &str) -> Option<&str> {
    if display_mode == "auto" {
        return None;
    }
    display_mode.split_once('@').map(|(wh, _)| wh)
}

/// Does `modes_text` (the verbatim contents of
/// `/sys/class/drm/card*-<connector>/modes` — one `WxH` per line, no refresh
/// column) list `want_wh`? Pure over fixture text so the wrong-panel case
/// (Dell asked for 2160) is testable without real DRM.
pub fn sysfs_modes_contains(modes_text: &str, want_wh: &str) -> bool {
    modes_text.lines().map(str::trim).any(|l| l == want_wh)
}

/// Rewrite a single-line `cmdline.txt`'s `video=<connector>:...` token to
/// match `kms_force`, preserving every other token, its position, and every
/// OTHER connector's `video=` token untouched. `kms_force == "none"` means
/// "remove the token for this connector, if present". Idempotent: running
/// this twice on its own output is a no-op, because a token that already
/// exists is replaced in place rather than moved to the end — the second run
/// finds the same token already correct and changes nothing.
pub fn reconcile_cmdline(cmdline_text: &str, connector: &str, kms_force: &str) -> Result<String, String> {
    let trimmed = cmdline_text.trim_end_matches(['\n', '\r']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err("cmdline.txt must be a single line".into());
    }
    if !is_valid_kms_force(kms_force) {
        return Err(format!("reconcile_cmdline: invalid kms_force {kms_force:?}"));
    }
    // A connectorless `video=` token forces ALL connectors; rewriting around
    // it would leave two overlapping forces for the kernel to arbitrate --
    // see connectorless_video_token's doc for why refusing beats guessing.
    if let Some(tok) = connectorless_video_token(trimmed) {
        return Err(format!(
            "cmdline.txt carries a connectorless token {tok:?}, which forces ALL connectors \
             -- refusing to reconcile per-connector video={connector}:... tokens around it \
             (the kernel would arbitrate two overlapping forces). Qualify or remove {tok:?} \
             by hand first"
        ));
    }
    let desired_token = if kms_force == DEFAULT_KMS_FORCE {
        None
    } else {
        Some(format!("video={connector}:{kms_force}"))
    };

    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for tok in trimmed.split_whitespace() {
        let is_ours = tok
            .strip_prefix("video=")
            .and_then(|rest| rest.split_once(':'))
            .is_some_and(|(conn, _)| conn == connector);
        if is_ours {
            replaced = true;
            if let Some(d) = &desired_token {
                out.push(d.clone());
            }
            // else: kms_force == "none" -- drop this token.
            continue;
        }
        out.push(tok.to_string());
    }
    if !replaced {
        if let Some(d) = desired_token {
            out.push(d);
        }
    }

    let result = out.join(" ");
    if result.trim().is_empty() {
        return Err(
            "reconcile_cmdline: result would be an empty cmdline.txt -- refusing to write it"
                .into(),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_valid_display_mode / is_valid_kms_force / is_valid_connector --

    #[test]
    fn display_mode_grammar() {
        for ok in [
            "auto",
            "3840x2160@30",
            "1x1@1",
            // Leading zeros are accepted -- positive_int only requires "all
            // digits, at least one nonzero", the same rule sidecar::is_valid_fps
            // applies to its numerator/denominator. Documented here rather than
            // left to be discovered by a future reader of the bad list below.
            "03840x2160@30",
        ] {
            assert!(is_valid_display_mode(ok), "{ok} should be valid");
        }
        for bad in [
            "",
            "Auto",
            "3840x2160",     // no @R -- mandatory, see docs
            "3840x2160@",    // empty R
            "x2160@30",      // empty W
            "3840x@30",      // empty H
            "3840X2160@30",  // uppercase X separator
            "3840x2160@0",   // R must be positive
            "3840x2160@-30", // negative R
            "3840x2160@30D", // D suffix is a kms_force thing, not display_mode
            "auto@30",
            // Bench-disproven forms an earlier revision accepted (dexpi4,
            // mpv 0.40, 2026-08-17 -- see is_valid_display_mode's docs):
            // rational fails mpv's option parser (-7, guaranteed restart
            // loop), decimal silently rounds to the integer vrefresh.
            "3840x2160@30000/1001",
            "2560x1440@59.95",
        ] {
            assert!(!is_valid_display_mode(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn kms_force_grammar() {
        for ok in ["none", "3840x2160@30", "3840x2160@30D", "1x1@1D"] {
            assert!(is_valid_kms_force(ok), "{ok} should be valid");
        }
        for bad in [
            "",
            "None",
            "auto",           // auto is a display_mode concept, not kms_force
            "3840x2160@29.97", // fractional refresh: kernel video= grammar has none
            "3840x2160",
            "3840x2160@",
            "3840x2160@30d",  // lowercase d is not the force suffix
            "3840x2160@30DD",
            "3840x2160@0",
            "3840x2160@0D",
        ] {
            assert!(!is_valid_kms_force(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn connector_grammar() {
        for ok in ["HDMI-A-1", "HDMI-A-2", "HDMI-A-0", "HDMI-A-10"] {
            assert!(is_valid_connector(ok), "{ok} should be valid");
        }
        for bad in ["", "HDMI-A-", "HDMI-A", "DP-1", "hdmi-a-1", "HDMI-A-1 "] {
            assert!(!is_valid_connector(bad), "{bad:?} should be invalid");
        }
    }

    // ---- ExhibitConfig::from_json ------------------------------------------

    #[test]
    fn parses_the_canonical_config() {
        let text = r#"{"display_mode":"3840x2160@30","kms_force":"3840x2160@30",
                        "connector":"HDMI-A-1","display":"Cam Link 4K",
                        "venue":"bench","note":"see PLAN.md F6"}"#;
        let c = ExhibitConfig::from_json(text).unwrap();
        assert_eq!(c.display_mode, "3840x2160@30");
        assert_eq!(c.kms_force, "3840x2160@30");
        assert_eq!(c.connector, "HDMI-A-1");
        assert_eq!(c.display.as_deref(), Some("Cam Link 4K"));
        assert_eq!(c.venue.as_deref(), Some("bench"));
        assert_eq!(c.note.as_deref(), Some("see PLAN.md F6"));
    }

    /// The .deb ships this exact file as `/etc/dex/exhibit.json`'s stock
    /// conffile content (Cargo.toml's `assets`). If a future grammar change
    /// ever made the shipped default itself invalid, every fresh install
    /// would refuse to start with no asset ever having been touched --
    /// pinned here rather than discovered on a bench.
    #[test]
    fn the_shipped_default_config_parses() {
        let text = include_str!("../deploy/exhibit.json.default");
        let c = ExhibitConfig::from_json(text).expect("shipped default must parse");
        assert_eq!(c.display_mode, "auto");
        assert_eq!(c.kms_force, "none");
        // The shipped default MUST name an asset. Without one, a stock install
        // now refuses at startup -- ExecStart no longer passes a path, and the
        // code deliberately has no fallback (see resolve_asset). This assert is
        // the gate on that: the conffile is the only thing standing between a
        // fresh install and "no asset" on first start.
        assert_eq!(c.asset.as_deref(), Some("/opt/dex/loop.265"));
        // ...and it must actually resolve, not merely parse.
        assert_eq!(
            resolve_asset(Some(&c), None, false).unwrap().path,
            "/opt/dex/loop.265"
        );
    }

    #[test]
    fn minimal_config_gets_defaults() {
        let c = ExhibitConfig::from_json(r#"{"display_mode":"auto"}"#).unwrap();
        assert_eq!(c.display_mode, "auto");
        assert_eq!(c.kms_force, DEFAULT_KMS_FORCE);
        assert_eq!(c.connector, DEFAULT_CONNECTOR);
        assert_eq!(c.display, None);
    }

    #[test]
    fn unknown_keys_are_refused_unlike_the_sidecar() {
        let e = ExhibitConfig::from_json(r#"{"display_mode":"auto","future_key":"x"}"#)
            .unwrap_err();
        assert!(e.contains("unknown key") && e.contains("future_key"), "{e}");
    }

    #[test]
    fn typo_in_kms_force_is_caught_not_silently_dropped() {
        // The exact field scenario the strict-schema decision defends against:
        // "kms_forse" (typo) must be a parse error, not an ignored key that
        // leaves the intended force unset.
        let e = ExhibitConfig::from_json(
            r#"{"display_mode":"3840x2160@30","kms_forse":"3840x2160@30"}"#,
        )
        .unwrap_err();
        assert!(e.contains("kms_forse"), "{e}");
    }

    #[test]
    fn missing_display_mode_is_refused_by_name() {
        let e = ExhibitConfig::from_json(r#"{"kms_force":"none"}"#).unwrap_err();
        assert!(e.contains("display_mode"), "{e}");
    }

    #[test]
    fn invalid_display_mode_and_kms_force_and_connector_are_refused() {
        assert!(ExhibitConfig::from_json(r#"{"display_mode":"nope"}"#).is_err());
        assert!(ExhibitConfig::from_json(
            r#"{"display_mode":"auto","kms_force":"nope"}"#
        )
        .is_err());
        assert!(ExhibitConfig::from_json(
            r#"{"display_mode":"auto","connector":"DP-1"}"#
        )
        .is_err());
    }

    #[test]
    fn display_mode_as_number_is_refused() {
        let e = ExhibitConfig::from_json(r#"{"display_mode":30}"#).unwrap_err();
        assert!(e.contains("string"), "{e}");
    }

    #[test]
    fn duplicate_keys_are_refused_inherited_from_the_sidecar_grammar() {
        let e = ExhibitConfig::from_json(
            r#"{"display_mode":"auto","display_mode":"3840x2160@30"}"#,
        )
        .unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
    }

    // ---- resolve_display ----------------------------------------------------

    fn cfg(display_mode: &str, kms_force: &str) -> ExhibitConfig {
        ExhibitConfig {
            asset: None,
            display_mode: display_mode.to_string(),
            kms_force: kms_force.to_string(),
            connector: DEFAULT_CONNECTOR.to_string(),
            display: None,
            venue: None,
            note: None,
        }
    }

    /// `cfg`, plus an asset — for the `resolve_asset` table.
    fn cfg_with_asset(asset: &str) -> ExhibitConfig {
        ExhibitConfig {
            asset: Some(asset.to_string()),
            ..cfg("auto", "none")
        }
    }

    #[test]
    fn deploy_path_takes_display_from_config() {
        let c = cfg("3840x2160@30", "3840x2160@30");
        let r = resolve_display(Some(&c), None, false).unwrap();
        assert_eq!(r.display_mode, "3840x2160@30");
        assert_eq!(r.kms_force, "3840x2160@30");
        assert_eq!(r.source, DisplaySource::Config);
    }

    #[test]
    fn agreeing_cli_mode_allowed_disagreeing_refused_naming_both() {
        let c = cfg("3840x2160@30", "none");
        assert!(resolve_display(Some(&c), Some("3840x2160@30"), false).is_ok());
        let e = resolve_display(Some(&c), Some("2560x1440@60"), false).unwrap_err();
        assert!(e.contains("3840x2160@30") && e.contains("2560x1440@60"), "{e}");
    }

    #[test]
    fn missing_config_is_refused_without_the_bench_flag() {
        let e = resolve_display(None, Some("3840x2160@30"), false).unwrap_err();
        assert!(e.contains("exhibit config"), "{e}");
        assert!(resolve_display(None, None, false).is_err());
    }

    #[test]
    fn bench_escape_hatch_ignores_config_entirely() {
        let c = cfg("2560x1440@60", "2560x1440@60");
        // CLI --mode wins even though it contradicts the config.
        let r = resolve_display(Some(&c), Some("3840x2160@30"), true).unwrap();
        assert_eq!(r.display_mode, "3840x2160@30");
        assert_eq!(r.source, DisplaySource::Bench);
        assert_eq!(r.kms_force, DEFAULT_KMS_FORCE);
    }

    #[test]
    fn bench_with_no_mode_defaults_to_auto_unlike_fps() {
        // Unlike resolve_fps, which REQUIRES --fps under the bench flag,
        // resolve_display treats a missing --mode as "auto" -- see the design
        // table: "no --mode means auto".
        let r = resolve_display(None, None, true).unwrap();
        assert_eq!(r.display_mode, "auto");
        assert_eq!(r.source, DisplaySource::Bench);
    }

    #[test]
    fn bench_with_invalid_mode_is_refused() {
        let e = resolve_display(None, Some("banana"), true).unwrap_err();
        assert!(e.contains("banana"), "{e}");
    }

    // ---- cmdline_video_token / check_cmdline_matches -------------------------

    #[test]
    fn cmdline_video_token_finds_only_the_named_connector() {
        let cl = "console=ttyS0 video=HDMI-A-1:3840x2160@30D quiet video=HDMI-A-2:1920x1080@60";
        assert_eq!(
            cmdline_video_token(cl, "HDMI-A-1"),
            Some("3840x2160@30D".to_string())
        );
        assert_eq!(
            cmdline_video_token(cl, "HDMI-A-2"),
            Some("1920x1080@60".to_string())
        );
        assert_eq!(cmdline_video_token(cl, "HDMI-A-3"), None);
    }

    #[test]
    fn cmdline_gate_none_expected_none_present_matches() {
        assert!(check_cmdline_matches("console=ttyS0 quiet", "HDMI-A-1", "none").is_ok());
    }

    #[test]
    fn cmdline_gate_none_expected_but_present_refuses_naming_the_fix() {
        let e = check_cmdline_matches(
            "video=HDMI-A-1:3840x2160@30",
            "HDMI-A-1",
            "none",
        )
        .unwrap_err();
        assert!(e.contains("dex-exhibit-apply") && e.contains("3840x2160@30"), "{e}");
    }

    #[test]
    fn cmdline_gate_wrong_mode_refuses_naming_both() {
        let e = check_cmdline_matches(
            "video=HDMI-A-1:3840x2160@30",
            "HDMI-A-1",
            "2560x1440@60",
        )
        .unwrap_err();
        assert!(e.contains("2560x1440@60") && e.contains("3840x2160@30"), "{e}");
    }

    #[test]
    fn cmdline_gate_wrong_connector_is_treated_as_absent() {
        // The configured connector's token is missing even though a DIFFERENT
        // connector's token is present -- must refuse "no token", not match.
        let e = check_cmdline_matches(
            "video=HDMI-A-2:1920x1080@60",
            "HDMI-A-1",
            "3840x2160@30",
        )
        .unwrap_err();
        assert!(e.contains("no video=") || e.contains("carries no"), "{e}");
    }

    #[test]
    fn cmdline_gate_refuses_a_connectorless_video_token() {
        // Kernel grammar also accepts `video=WxH@R` with no connector, which
        // forces ALL connectors. Previously invisible to the gate (reported
        // as "no video= token") -- it must refuse, naming the token.
        let e = check_cmdline_matches(
            "console=ttyS0 video=1920x1080@60 quiet",
            "HDMI-A-1",
            "3840x2160@30",
        )
        .unwrap_err();
        assert!(e.contains("video=1920x1080@60") && e.contains("ALL connectors"), "{e}");
        // Even when the per-connector expectation is "none": the global
        // force still binds our connector, so "matches" would be a lie.
        let e = check_cmdline_matches("video=1920x1080@60", "HDMI-A-1", "none").unwrap_err();
        assert!(e.contains("connectorless"), "{e}");
    }

    #[test]
    fn connectorless_video_token_ignores_per_connector_tokens() {
        assert_eq!(connectorless_video_token("video=HDMI-A-1:3840x2160@30 quiet"), None);
        assert_eq!(
            connectorless_video_token("quiet video=1024x768"),
            Some("video=1024x768".to_string())
        );
        assert_eq!(connectorless_video_token("console=ttyS0 quiet"), None);
    }

    #[test]
    fn cmdline_gate_expected_present_matches() {
        assert!(check_cmdline_matches(
            "root=/dev/mmcblk0p2 video=HDMI-A-1:3840x2160@30D rootwait",
            "HDMI-A-1",
            "3840x2160@30D",
        )
        .is_ok());
    }

    // ---- mode_resolution / sysfs_modes_contains ------------------------------

    #[test]
    fn mode_resolution_strips_the_refresh_auto_has_none() {
        assert_eq!(mode_resolution("3840x2160@30"), Some("3840x2160"));
        assert_eq!(mode_resolution("auto"), None);
    }

    #[test]
    fn sysfs_modes_contains_checks_whole_line_matches() {
        let modes = "3840x2160\n3840x2160\n1920x1080\n1024x768\n";
        assert!(sysfs_modes_contains(modes, "3840x2160"));
        assert!(sysfs_modes_contains(modes, "1024x768"));
        assert!(!sysfs_modes_contains(modes, "2560x1440"));
        // Multi-card glob output concatenated is still just lines.
        let two_cards = "3840x2160\n1920x1080\n2560x1440\n1920x1080\n";
        assert!(sysfs_modes_contains(two_cards, "2560x1440"));
        assert!(!sysfs_modes_contains(two_cards, "7680x4320"));
    }

    #[test]
    fn sysfs_modes_contains_trims_whitespace() {
        assert!(sysfs_modes_contains(" 3840x2160 \r\n", "3840x2160"));
    }

    // ---- reconcile_cmdline ---------------------------------------------------

    #[test]
    fn reconcile_adds_a_token_when_none_existed() {
        let out = reconcile_cmdline(
            "console=ttyS0 root=/dev/mmcblk0p2 rootwait quiet",
            "HDMI-A-1",
            "3840x2160@30D",
        )
        .unwrap();
        assert_eq!(
            out,
            "console=ttyS0 root=/dev/mmcblk0p2 rootwait quiet video=HDMI-A-1:3840x2160@30D"
        );
    }

    #[test]
    fn reconcile_replaces_in_place_preserving_position_and_order() {
        let out = reconcile_cmdline(
            "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait quiet",
            "HDMI-A-1",
            "2560x1440@60",
        )
        .unwrap();
        assert_eq!(
            out,
            "console=ttyS0 video=HDMI-A-1:2560x1440@60 rootwait quiet"
        );
    }

    #[test]
    fn reconcile_removes_the_token_for_none() {
        let out = reconcile_cmdline(
            "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait",
            "HDMI-A-1",
            "none",
        )
        .unwrap();
        assert_eq!(out, "console=ttyS0 rootwait");
    }

    #[test]
    fn reconcile_leaves_other_connectors_video_tokens_untouched() {
        let out = reconcile_cmdline(
            "video=HDMI-A-2:1920x1080@60 video=HDMI-A-1:3840x2160@30 quiet",
            "HDMI-A-1",
            "none",
        )
        .unwrap();
        assert_eq!(out, "video=HDMI-A-2:1920x1080@60 quiet");
    }

    #[test]
    fn reconcile_is_idempotent() {
        let once = reconcile_cmdline(
            "console=ttyS0 rootwait",
            "HDMI-A-1",
            "3840x2160@30D",
        )
        .unwrap();
        let twice = reconcile_cmdline(&once, "HDMI-A-1", "3840x2160@30D").unwrap();
        assert_eq!(once, twice);

        // And the "no change" case: apply the SAME force that is already
        // present -- dex-exhibit-apply's real first-run-on-dexpi4 scenario.
        let already = "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait";
        let out = reconcile_cmdline(already, "HDMI-A-1", "3840x2160@30").unwrap();
        assert_eq!(out, already);
    }

    #[test]
    fn reconcile_refuses_a_connectorless_video_token() {
        // Previously the reconciler would leave the global force in place and
        // append its own per-connector token -- two overlapping forces for
        // the kernel to arbitrate. Refuse instead, naming the token.
        let e = reconcile_cmdline("console=ttyS0 video=1024x768 rootwait", "HDMI-A-1", "3840x2160@30")
            .unwrap_err();
        assert!(e.contains("video=1024x768") && e.contains("by hand"), "{e}");
        // Same refusal on the removal direction (kms_force=none).
        assert!(reconcile_cmdline("video=1024x768", "HDMI-A-1", "none").is_err());
    }

    #[test]
    fn reconcile_no_op_when_none_requested_and_none_present() {
        let out = reconcile_cmdline("console=ttyS0 rootwait", "HDMI-A-1", "none").unwrap();
        assert_eq!(out, "console=ttyS0 rootwait");
    }

    #[test]
    fn reconcile_rejects_multi_line_input() {
        assert!(reconcile_cmdline("a b\nc d", "HDMI-A-1", "none").is_err());
    }

    #[test]
    fn reconcile_tolerates_a_trailing_newline() {
        // cmdline.txt conventionally has no trailing newline, but a file an
        // editor "fixed" by adding one must not be treated as multi-line.
        let out = reconcile_cmdline("console=ttyS0\n", "HDMI-A-1", "none").unwrap();
        assert_eq!(out, "console=ttyS0");
    }

    #[test]
    fn reconcile_refuses_an_empty_result() {
        let e = reconcile_cmdline("video=HDMI-A-1:3840x2160@30", "HDMI-A-1", "none").unwrap_err();
        assert!(e.contains("empty"), "{e}");
    }

    #[test]
    fn reconcile_rejects_an_invalid_kms_force() {
        assert!(reconcile_cmdline("console=ttyS0", "HDMI-A-1", "banana").is_err());
    }

    // ---- the format dispatch -------------------------------------------
    //
    // The rule under test is "the extension decides", so these check the
    // DISPATCH, not the parsers. The parsers get their own sections below.

    #[test]
    fn extension_decides_the_parser() {
        assert_eq!(
            ConfigFormat::from_path("/etc/dex/exhibit.json").unwrap(),
            ConfigFormat::Json
        );
        assert_eq!(
            ConfigFormat::from_path("/etc/dex/exhibit.yaml").unwrap(),
            ConfigFormat::Yaml
        );
        assert_eq!(
            ConfigFormat::from_path("/etc/dex/exhibit.yml").unwrap(),
            ConfigFormat::Yaml
        );
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert_eq!(
            ConfigFormat::from_path("/tmp/EXHIBIT.JSON").unwrap(),
            ConfigFormat::Json
        );
        assert_eq!(
            ConfigFormat::from_path("/tmp/Exhibit.Yaml").unwrap(),
            ConfigFormat::Yaml
        );
    }

    #[test]
    fn unknown_or_absent_extension_refuses_rather_than_defaulting() {
        for p in [
            "/etc/dex/exhibit",     // no extension at all
            "/etc/dex/exhibit.txt", // an extension, but not one of ours
            "/etc/dex/.json",       // a DOTFILE named .json -- no extension
            "/etc/dex/exhibit.json.bak", // the backup, not the config
        ] {
            let e = ConfigFormat::from_path(p).unwrap_err();
            assert!(e.contains(".json") && e.contains(".yaml"), "{p}: {e}");
        }
    }

    /// THE point of the whole dispatch: YAML syntax inside a file named
    /// `.json` must be refused, even though a YAML parser would accept it
    /// happily. A `.json` file that only `dex-loop` can read is a broken
    /// promise to `jq` and everything else downstream.
    #[test]
    fn yaml_syntax_in_a_json_file_is_refused() {
        let yaml_text = "display_mode: 3840x2160@30\nkms_force: none\n";
        // The YAML parser accepts it, proving the input is valid YAML...
        assert!(ExhibitConfig::parse(yaml_text, ConfigFormat::Yaml).is_ok());
        // ...and the JSON parser must still refuse it under a .json name.
        assert!(ExhibitConfig::parse(yaml_text, ConfigFormat::Json).is_err());
    }

    /// The converse, which must NOT be an error: JSON is a subset of YAML, so
    /// a machine that writes strict JSON into a `.yaml` file still parses.
    /// This is what lets an ingest tool emit one format for both names.
    #[test]
    fn json_text_parses_under_the_yaml_parser_too() {
        let json_text = r#"{"display_mode":"3840x2160@30","kms_force":"none"}"#;
        let as_json = ExhibitConfig::parse(json_text, ConfigFormat::Json).unwrap();
        let as_yaml = ExhibitConfig::parse(json_text, ConfigFormat::Yaml).unwrap();
        assert_eq!(as_json, as_yaml);
    }

    /// Equivalence: the same config in either format produces the same
    /// struct, byte for byte. This is the test that would fail first if the
    /// two paths ever stopped sharing `from_pairs`.
    #[test]
    fn both_formats_agree_on_a_full_config() {
        let json = ExhibitConfig::from_json(
            r#"{"display_mode":"3840x2160@30","kms_force":"3840x2160@30D",
                "connector":"HDMI-A-2","display":"Elgato Cam Link 4K",
                "venue":"gallery east wall","note":"vc4 builds no 4K mode unforced"}"#,
        )
        .unwrap();
        let yaml = ExhibitConfig::from_yaml(
            "# the same thing, with the comments JSON cannot carry\n\
             display_mode: 3840x2160@30\n\
             kms_force: 3840x2160@30D    # trailing D: force `connected`\n\
             connector: HDMI-A-2\n\
             display: Elgato Cam Link 4K\n\
             venue: gallery east wall\n\
             note: vc4 builds no 4K mode unforced\n",
        )
        .unwrap();
        assert_eq!(json, yaml);
    }

    /// Validation is shared, so a YAML file gets the JSON path's messages --
    /// including the strict-schema unknown-key refusal that a typo'd
    /// `kms_forse` must produce in either format.
    #[test]
    fn yaml_inherits_the_strict_schema_and_the_grammars() {
        let e = ExhibitConfig::from_yaml("display_mode: auto\nkms_forse: none\n").unwrap_err();
        assert!(e.contains("kms_forse") && e.contains("unknown key"), "{e}");

        let e = ExhibitConfig::from_yaml("display_mode: 3840x2160@29.97\n").unwrap_err();
        assert!(e.contains("invalid display_mode"), "{e}");

        let e = ExhibitConfig::from_yaml("kms_force: none\n").unwrap_err();
        assert!(e.contains("display_mode"), "{e}");
    }

    // ---- the YAML subset ------------------------------------------------

    /// yaml-rust2's loader errors on a duplicate key rather than last-wins.
    /// That is the ONE rule the JSON side needed a hand-written serde visitor
    /// for, so it is load-bearing that the YAML side gets it for free --
    /// pinned by test, because it is a property of the dependency and would
    /// otherwise silently regress on a version bump.
    #[test]
    fn yaml_duplicate_key_refused() {
        let e =
            ExhibitConfig::from_yaml("display_mode: auto\ndisplay_mode: 3840x2160@30\n").unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
    }

    /// A bare `true`/`false` is a boolean, refused with the quoting fix.
    #[test]
    fn yaml_bare_true_false_are_booleans_and_refused() {
        for text in [
            "display_mode: true\n",
            "display_mode: auto\nnote: FALSE\n",
        ] {
            let e = ExhibitConfig::from_yaml(text).unwrap_err();
            assert!(e.contains("BOOLEAN"), "{text:?}: {e}");
            assert!(e.contains("Quote the value"), "{text:?}: {e}");
        }
    }

    /// The OTHER half, and the one that is easy to get wrong from memory:
    /// yaml-rust2 0.11 resolves close to the YAML **1.2 core schema**, so the
    /// "Norway problem" does NOT apply -- `no` stays the string `"no"` and is
    /// refused by the kms_force GRAMMAR, not by the boolean branch. Asserted
    /// on the message so that a future version adopting 1.1 resolution (which
    /// would make `kms_force: no` mean `false`) fails here rather than
    /// changing what a deployed config means.
    #[test]
    fn yaml_bare_no_stays_a_string_under_the_1_2_core_schema() {
        let e = ExhibitConfig::from_yaml("display_mode: auto\nkms_force: no\n").unwrap_err();
        assert!(
            e.contains("invalid kms_force \"no\""),
            "expected the grammar refusal for the STRING \"no\", not a boolean one: {e}"
        );
        assert!(!e.contains("BOOLEAN"), "{e}");
        // ...and stating it properly is the fix.
        let ok = ExhibitConfig::from_yaml("display_mode: auto\nkms_force: none\n").unwrap();
        assert_eq!(ok.kms_force, "none");
    }

    /// Where yaml-rust2 DEPARTS from the 1.2 core schema, pinned so the docs
    /// cannot quietly become wrong: core lists `null | Null | NULL | ~ | empty`
    /// as null, but this library's `from_str` matches only `""`, `"~"` and
    /// `"null"` — case-sensitively. So the capitalised spellings arrive as
    /// ordinary strings.
    ///
    /// Harmless here (only the informational keys could carry such a value,
    /// and reading it literally is the friendlier of the two options), but
    /// "it implements the core schema" is a nearly-true sentence a future
    /// reader would rely on, and this is the test that keeps it honest.
    #[test]
    fn yaml_null_resolution_is_case_sensitive_unlike_the_1_2_core_schema() {
        for null_spelling in ["null", "~", ""] {
            let e = ExhibitConfig::from_yaml(&format!("display_mode: auto\nnote: {null_spelling}\n"))
                .unwrap_err();
            assert!(e.contains("null"), "{null_spelling:?}: {e}");
        }
        for string_spelling in ["Null", "NULL"] {
            let c = ExhibitConfig::from_yaml(&format!(
                "display_mode: auto\nnote: {string_spelling}\n"
            ))
            .expect("core would call this null; yaml-rust2 does not");
            assert_eq!(c.note.as_deref(), Some(string_spelling));
        }
    }

    /// Scalar resolution happens BEFORE this crate's subset check, so it stays
    /// observable — and both formats must land in the same place. A bare
    /// number resolves to an integer and is then refused for a string-only
    /// key, identically to the JSON spelling of the same thing.
    #[test]
    fn a_bare_number_is_refused_the_same_way_in_both_formats() {
        let y = ExhibitConfig::from_yaml("display_mode: auto\nvenue: 2026\n").unwrap_err();
        let j = ExhibitConfig::from_json(r#"{"display_mode":"auto","venue":2026}"#).unwrap_err();
        assert_eq!(y, j, "the two formats must refuse identically");
        assert!(y.contains("venue must be a string"), "{y}");
        // ...and quoting is the fix in both.
        assert_eq!(
            ExhibitConfig::from_yaml("display_mode: auto\nvenue: \"2026\"\n").unwrap(),
            ExhibitConfig::from_json(r#"{"display_mode":"auto","venue":"2026"}"#).unwrap()
        );
    }

    #[test]
    fn yaml_nesting_lists_and_null_are_refused() {
        let nested = ExhibitConfig::from_yaml("display_mode: auto\nnote:\n  a: b\n").unwrap_err();
        assert!(nested.contains("nested mapping"), "{nested}");
        let list = ExhibitConfig::from_yaml("display_mode: auto\nnote:\n  - a\n").unwrap_err();
        assert!(list.contains("a list"), "{list}");
        let null = ExhibitConfig::from_yaml("display_mode: auto\nnote:\n").unwrap_err();
        assert!(null.contains("null"), "{null}");
    }

    #[test]
    fn yaml_multiple_documents_refused_rather_than_taking_the_first() {
        let e = ExhibitConfig::from_yaml(
            "display_mode: auto\n---\ndisplay_mode: 3840x2160@30\n",
        )
        .unwrap_err();
        assert!(e.contains("2 documents"), "{e}");
    }

    #[test]
    fn yaml_empty_or_comment_only_says_so_rather_than_blaming_display_mode() {
        for text in ["", "   \n", "# just a comment\n"] {
            let e = ExhibitConfig::from_yaml(text).unwrap_err();
            assert!(e.contains("no document"), "{text:?}: {e}");
        }
    }

    #[test]
    fn yaml_top_level_scalar_or_list_refused() {
        let e = ExhibitConfig::from_yaml("just a string\n").unwrap_err();
        assert!(e.contains("top level"), "{e}");
        let e = ExhibitConfig::from_yaml("- a\n- b\n").unwrap_err();
        assert!(e.contains("top level") && e.contains("a list"), "{e}");
    }

    /// An integer VALUE parses (the shared mapper then refuses it for these
    /// particular keys, in the same words the JSON path uses) -- but a
    /// NEGATIVE one is out of range for the format itself.
    #[test]
    fn yaml_integers_follow_the_json_subset() {
        let e = ExhibitConfig::from_yaml("display_mode: 30\n").unwrap_err();
        assert!(e.contains("display_mode must be a string"), "{e}");
        let e = ExhibitConfig::from_yaml("display_mode: auto\nnote: -1\n").unwrap_err();
        assert!(e.contains("negative"), "{e}");
    }

    // ---- default-config discovery ---------------------------------------

    #[test]
    fn one_default_config_is_picked_none_is_none() {
        assert_eq!(pick_default_config(&[]).unwrap(), None);
        assert_eq!(
            pick_default_config(&["/etc/dex/exhibit.yaml"]).unwrap(),
            Some("/etc/dex/exhibit.yaml")
        );
        assert_eq!(
            pick_default_config(&["/etc/dex/exhibit.json"]).unwrap(),
            Some("/etc/dex/exhibit.json")
        );
    }

    /// Two configs at once refuses, naming both -- never "YAML wins".
    /// Precedence here would let an operator edit one file all afternoon
    /// while the player reads the other.
    #[test]
    fn two_default_configs_refuse_naming_both() {
        let e = pick_default_config(&DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(e.contains("exhibit.yaml") && e.contains("exhibit.json"), "{e}");
        assert!(e.contains("--exhibit-config"), "{e}");
        // The likely cause, named: writing exhibit.yaml leaves the shipped
        // conffile beside it, so the operator did the documented thing and
        // still got refused. Without this the message reads like a bug.
        assert!(
            e.contains("sudo rm /etc/dex/exhibit.json"),
            "must name the leftover conffile as the fix: {e}"
        );
    }

    /// ...and that hint is CONDITIONAL, not glued on: a collision between two
    /// non-shipped files must not tell the operator to remove a package file
    /// that has nothing to do with it.
    #[test]
    fn the_leftover_conffile_hint_only_appears_when_a_json_is_involved() {
        let e = pick_default_config(&["/srv/a.yaml", "/srv/b.yml"]).unwrap_err();
        assert!(!e.contains("sudo rm"), "{e}");
    }

    // ---- resolve_asset: the whole decision table -------------------------

    #[test]
    fn deploy_path_takes_the_asset_from_the_config() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), None, false).unwrap();
        assert_eq!(r.path, "/opt/dex/spring.265");
        assert_eq!(r.source, AssetSource::Config);
    }

    #[test]
    fn an_agreeing_cli_path_cross_checks_and_the_config_still_binds() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), Some("/opt/dex/spring.265"), false).unwrap();
        assert_eq!(r.source, AssetSource::Config);
    }

    #[test]
    fn a_contradicting_cli_path_is_refused_naming_both() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let e = resolve_asset(Some(&c), Some("/opt/dex/autumn.265"), false).unwrap_err();
        assert!(e.contains("spring.265") && e.contains("autumn.265"), "{e}");
    }

    /// A config with no `asset` key still accepts a hand-given path -- the
    /// one-off case (try another file on a deployed device without editing
    /// /etc), reported as CLI-sourced so the journal cannot be misread.
    #[test]
    fn a_cli_path_works_when_the_config_names_no_asset() {
        let c = cfg("auto", "none");
        let r = resolve_asset(Some(&c), Some("/opt/dex/try.265"), false).unwrap();
        assert_eq!(r.source, AssetSource::Cli);
    }

    /// THE fail-closed row: nothing names an asset, so nothing is guessed --
    /// specifically NOT /opt/dex/loop.265, which would silently play last
    /// season's artwork for someone who mistyped the key.
    #[test]
    fn no_asset_anywhere_refuses_rather_than_defaulting_to_loop_265() {
        let c = cfg("auto", "none");
        let e = resolve_asset(Some(&c), None, false).unwrap_err();
        assert!(e.contains("no asset"), "{e}");
        // ...and it names the exact line to add, in both formats.
        assert!(e.contains("asset: /opt/dex/loop.265"), "{e}");
        assert!(e.contains(r#""asset": "/opt/dex/loop.265""#), "{e}");
        // Also with NO config at all (that case refuses earlier, at
        // resolve_display -- but this function must not invent a path either).
        assert!(resolve_asset(None, None, false).is_err());
    }

    #[test]
    fn bench_takes_the_cli_path_and_ignores_the_config() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), Some("/tmp/bench.265"), true).unwrap();
        assert_eq!(r.path, "/tmp/bench.265");
        assert_eq!(r.source, AssetSource::Cli);
    }

    #[test]
    fn bench_without_a_path_refuses_rather_than_falling_back_to_the_config() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let e = resolve_asset(Some(&c), None, true).unwrap_err();
        assert!(e.contains("command line"), "{e}");
    }

    // ---- the asset grammar ----------------------------------------------

    #[test]
    fn asset_must_be_an_absolute_path() {
        assert!(is_valid_asset("/opt/dex/loop.265"));
        assert!(is_valid_asset("/srv/art/Karte–Süd.265")); // non-ASCII is fine
        // A relative path would resolve against the service's working
        // directory (`/`), i.e. somewhere the operator did not type.
        assert!(!is_valid_asset("loop.265"));
        assert!(!is_valid_asset("./loop.265"));
        assert!(!is_valid_asset(""));
        assert!(!is_valid_asset("/opt/dex/")); // a directory, not a file
    }

    /// The asset path is printed into the journal at every start, and the
    /// journal is the only diagnostic channel a gallery device has -- a
    /// newline in it would forge a second log line.
    #[test]
    fn asset_with_a_control_character_is_refused() {
        assert!(!is_valid_asset("/opt/dex/loop.265\ndex-loop: all fine here"));
        assert!(!is_valid_asset("/opt/dex/loop\t.265"));
    }

    #[test]
    fn asset_parses_from_both_formats_and_is_grammar_checked() {
        let j = ExhibitConfig::from_json(
            r#"{"asset":"/opt/dex/spring.265","display_mode":"auto"}"#,
        )
        .unwrap();
        let y = ExhibitConfig::from_yaml("asset: /opt/dex/spring.265\ndisplay_mode: auto\n").unwrap();
        assert_eq!(j, y);
        assert_eq!(j.asset.as_deref(), Some("/opt/dex/spring.265"));

        let e = ExhibitConfig::from_yaml("asset: loop.265\ndisplay_mode: auto\n").unwrap_err();
        assert!(e.contains("invalid asset"), "{e}");
    }

    // ---- load_exhibit_config, against a real temp directory --------------
    //
    // `defaults` is injectable, so the discovery policy both binaries share is
    // testable here rather than only through the CLI on a machine that happens
    // to have /etc/dex.

    /// A unique temp path per call site, so tests never collide with each
    /// other or with a previous run's leftovers.
    fn tmp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("dex-exhibit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn load_finds_the_yaml_default_and_parses_it_as_yaml() {
        let y = tmp("load-yaml.yaml");
        std::fs::write(&y, "display_mode: 3840x2160@30 # venue panel\n").unwrap();
        let (cfg, path) = load_exhibit_config(None, &[&y, &tmp("load-yaml-absent.json")])
            .unwrap()
            .unwrap();
        assert_eq!(cfg.display_mode, "3840x2160@30");
        assert_eq!(path, y);
        let _ = std::fs::remove_file(&y);
    }

    #[test]
    fn load_returns_none_when_no_default_exists() {
        let found = load_exhibit_config(
            None,
            &[&tmp("load-absent.yaml"), &tmp("load-absent.json")],
        )
        .unwrap();
        assert!(found.is_none(), "{found:?}");
    }

    /// The whole reason both binaries call this: with both names present it
    /// refuses instead of picking, so `dex-exhibit-apply` can never reconcile
    /// the cmdline against a file `dex-loop` will not read.
    #[test]
    fn load_refuses_when_both_defaults_exist() {
        let (j, y) = (tmp("load-both.json"), tmp("load-both.yaml"));
        std::fs::write(&j, r#"{"display_mode":"auto"}"#).unwrap();
        std::fs::write(&y, "display_mode: auto\n").unwrap();
        let e = load_exhibit_config(None, &[&y, &j]).unwrap_err();
        assert!(e.contains("2 exhibit configs"), "{e}");
        let _ = std::fs::remove_file(&j);
        let _ = std::fs::remove_file(&y);
    }

    /// An explicitly named missing file must NOT borrow the "no exhibit
    /// config, create the default" message -- the operator named a different
    /// path, and telling them to create /etc/dex/exhibit.json is wrong advice.
    #[test]
    fn load_names_the_explicit_path_when_it_is_missing() {
        let p = tmp("load-explicitly-absent.json");
        let _ = std::fs::remove_file(&p);
        let e = load_exhibit_config(Some(&p), &DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(e.contains(&p), "{e}");
        assert!(!e.contains("Create /etc/dex"), "{e}");
    }

    /// A config whose name promises neither format refuses at the dispatch,
    /// before any parse is attempted -- even though its CONTENTS would parse
    /// perfectly well as either.
    #[test]
    fn load_refuses_an_unrecognised_extension_even_with_valid_contents() {
        let p = tmp("load-nameless.conf");
        std::fs::write(&p, r#"{"display_mode":"auto"}"#).unwrap();
        let e = load_exhibit_config(Some(&p), &DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(e.contains("cannot tell the format"), "{e}");
        let _ = std::fs::remove_file(&p);
    }
}
