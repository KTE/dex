//! The exhibit config: its grammar, its parsers, and the decisions it feeds.
//!
//! The config names the video this installation plays and the display it plays on. It sits
//! next to the video in the assets directory, and dexd refuses to start without one. The file
//! extension picks the parser: `exhibit.json` is strict JSON, `exhibit.yaml` (or `.yml`) is YAML.
//!
//! ```yaml
//! asset: loop.265               # the video, beside this file
//! display_mode: 3840x2160@30    # required; what mpv is asked for
//! kms_force: 3840x2160@30       # what the kernel cmdline must carry; optional, default none
//! connector: HDMI-A-1           # optional, and the default
//! # display, venue and note are informational; nothing in the player reads them
//! ```
//!
//! [`ExhibitConfig::from_pairs`] defines the whole schema for both formats and refuses an
//! unknown key. Everything but [`load_exhibit_config`] is pure: the grammars, config
//! discovery, the [`resolve_display`] and [`resolve_asset`] tables, the cmdline check and
//! rewrite and the sysfs pre-flight all test without libmpv or real hardware.
//!
//! See docs/design/exhibit-config.md#config-location and docs/design/architecture.md#crate-layout.

use crate::sidecar::{parse_flat_json, Value};
use yaml_rust2::{Event, Yaml, YamlLoader};

/// The path the messages name when no exhibit config exists: the file an
/// operator creates, in the assets directory, next to the video it names.
/// The package installs no exhibit config, so this path names nothing on
/// disk until someone writes it.
pub const DEFAULT_EXHIBIT_CONFIG_PATH: &str = "/opt/dex/exhibit.yaml";

/// Where the player looks when `--exhibit-config` is not given, in order.
///
/// Both names sit in the assets directory, so the dex card in a computer shows
/// the video, its sidecar and the exhibit config together. YAML is searched
/// first, which only fixes which name a refusal calls the intended one:
/// [`pick_default_config`] refuses outright when both files exist.
/// See docs/design/exhibit-config.md#config-location.
pub const DEFAULT_EXHIBIT_CONFIG_PATHS: [&str; 2] =
    ["/opt/dex/exhibit.yaml", "/opt/dex/exhibit.json"];

/// Choose the default config among those that exist on disk.
///
/// `existing` is the subset of [`DEFAULT_EXHIBIT_CONFIG_PATHS`] that exists, in
/// that array's order; main.rs makes the `Path::exists` calls, so this stays
/// pure. One existing path is returned as the config. None returns `None`, and
/// [`resolve_display`] states the "no exhibit config" refusal. Two or more are
/// refused: nothing here can tell which file the operator meant, and choosing
/// by precedence would hide the question.
/// See docs/design/exhibit-config.md#config-location.
pub fn pick_default_config<'a>(existing: &[&'a str]) -> Result<Option<&'a str>, String> {
    match existing {
        [] => Ok(None),
        [only] => Ok(Some(only)),
        several => {
            // Name the likely cause as well as the rule: switching to YAML
            // leaves the older exhibit.json beside the new file, so the
            // operator did one correct thing and still got a refusal.
            let json_copies: Vec<&str> = several
                .iter()
                .copied()
                .filter(|p| p.ends_with(".json"))
                .collect();
            let hint = if json_copies.is_empty() {
                String::new()
            } else {
                format!(
                    " If you have just switched to YAML, delete the older file: sudo rm {}.",
                    json_copies.join(" ")
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

/// Is `t` a non-empty run of decimal digits with at least one nonzero digit —
/// the "positive integer" shape the display mode and force grammars share.
fn positive_int(t: &str) -> bool {
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && t.bytes().any(|b| b != b'0')
}

/// Is `t` a non-empty run of decimal digits? Leading zeros and an all-zero
/// value are both allowed: connector numbering is not a rate, and `0` is a
/// legitimate index on some drivers.
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

/// `display_mode` grammar: `"auto"`, or `"WxH@R"` with W, H and R positive
/// integers.
///
/// The `@R` part is mandatory and must be an integer. A decimal such as
/// `@29.97` plays the rounded integer mode without saying so; a rational such
/// as `@30000/1001` fails mpv's option parser at startup; and `"3840x2160"` on
/// its own lets mpv pick among same-resolution timings by list order. An
/// integer refresh the connector does not offer fails later, at video-output
/// init, since the pre-flight can check only the resolution half (see
/// [`mode_resolution`]).
/// See docs/design/exhibit-config.md#display-mode.
pub fn is_valid_display_mode(s: &str) -> bool {
    if s == "auto" {
        return true;
    }
    match split_mode(s) {
        Some((w, h, r)) => positive_int(w) && positive_int(h) && positive_int(r),
        None => false,
    }
}

/// `kms_force` grammar: `"none"`, or `"WxH@R"`/`"WxH@RD"` with W, H and R
/// positive integers. R is an integer here because the kernel's `video=`
/// cmdline grammar has no fractional refresh. The trailing `D` makes the
/// connector read as `connected` even when nothing is attached yet; it is a
/// suffix on the whole token, not part of the refresh number.
/// See docs/design/exhibit-config.md#display-mode.
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

/// `asset` grammar: a path to a file, absolute or relative, with no trailing
/// slash and no control characters.
///
/// A relative path is resolved against the directory the config file is in
/// (see [`asset_in_config_dir`]), so `asset: loop.265` beside
/// `/opt/dex/exhibit.yaml` names `/opt/dex/loop.265`; the service's working
/// directory never enters into it. Control characters are refused because this
/// string is printed into the system log at every start, where a newline would
/// forge a second log line.
///
/// Existence and file extension are not checked here: main.rs opens the file
/// and produces the read error, which says more than a grammar refusal would.
pub fn is_valid_asset(s: &str) -> bool {
    !s.is_empty() && !s.ends_with('/') && !s.chars().any(|c| c.is_ascii_control())
}

/// The directory `config_path` sits in — what a relative `asset` resolves
/// against. Pure string work, so it needs no filesystem and no real config.
pub fn config_dir(config_path: &str) -> &str {
    match config_path.rfind('/') {
        Some(0) => "/",
        Some(i) => &config_path[..i],
        None => ".",
    }
}

/// Resolve an `asset` value against the directory its config file is in.
///
/// An absolute `asset` is returned unchanged. A relative one is joined to
/// `dir`, including a `../` prefix, which the kernel resolves at open time:
/// `../x` beside `/opt/dex/exhibit.yaml` opens `/opt/dex/../x`. The join is
/// textual, so the path printed in the system log shows the two halves the
/// operator wrote.
pub fn asset_in_config_dir(dir: &str, asset: &str) -> String {
    if asset.starts_with('/') {
        asset.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{asset}")
    } else {
        format!("{dir}/{asset}")
    }
}

/// `connector` grammar: `"HDMI-A-<n>"`, n a run of digits. The same value
/// spells the `video=<connector>:...` token dex-exhibit-apply writes and the
/// `/sys/class/drm/card*-<connector>` glob the pre-flight reads, so a wrong
/// value accepted here would fail far from where it was typed.
pub fn is_valid_connector(s: &str) -> bool {
    match s.strip_prefix("HDMI-A-") {
        Some(n) => digits_only(n),
        None => false,
    }
}

/// Which parser reads an exhibit config, decided by the file extension.
///
/// YAML is a superset of JSON, so parsing everything as YAML would accept
/// comments, anchors and unquoted keys inside a file named `.json`, and that
/// file would then break `jq` and every other tool that selects its parser
/// by the extension.
/// The dispatch therefore sits at the outermost layer, where no convenience
/// helper can bypass it, and the `.json` path runs on a real JSON parser.
///
/// Everything after tree-building is shared: both parsers produce the
/// `Vec<(String, Value)>` flat map [`crate::sidecar::parse_flat_json`]
/// produces, and [`ExhibitConfig::from_pairs`] does all the mapping and
/// validation for both.
/// See docs/design/exhibit-config.md#file-format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Yaml,
}

impl ConfigFormat {
    /// Decide the format from a path's extension, case-insensitively: an
    /// editor may have written `EXHIBIT.YAML`, and the promise the extension
    /// makes is the same either way.
    ///
    /// An unrecognised or absent extension is refused instead of defaulting to
    /// one of the parsers. `Path::extension` decides, so a dotfile named
    /// `.json` has no extension and is refused — the same reading of a file
    /// name every other tool applies.
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

/// Parse `text` as a single flat YAML mapping under the same subset
/// [`crate::sidecar::parse_flat_json`] enforces for JSON — strings and
/// non-negative integers only, one level deep, no duplicate keys — returning
/// its key/value pairs in source order.
///
/// Anchors and aliases are refused before the load, by
/// [`refuse_anchors_and_aliases`].
///
/// The subset keeps the two formats interchangeable: a YAML feature refused
/// here is one that could not be written in the `.json` form of the same
/// config. So a bare `true`/`false` (a boolean, not the text — the message
/// names the quoting fix), a decimal such as `29.97`, and a `---`-separated
/// stream of several documents are refused rather than coerced; `yaml-rust2`
/// itself errors on a duplicate key.
///
/// `yaml-rust2` 0.11 resolves scalars close to the YAML 1.2 core schema and
/// offers no way to select another. Its treatment of `no`, `null` and `Null`
/// is locked in by tests, since a version bump that changed it would change
/// what a deployed config means.
/// See docs/design/exhibit-config.md#file-format.
pub fn parse_flat_yaml(text: &str) -> Result<Vec<(String, Value)>, String> {
    refuse_anchors_and_aliases(text)?;
    let docs = YamlLoader::load_from_str(text).map_err(|e| format!("exhibit config YAML: {e}"))?;
    let doc =
        match docs.len() {
            // load_from_str returns zero documents for empty or comment-only
            // input. Treated as a parse failure rather than an empty config:
            // an empty config would fail on the missing `display_mode` key and
            // report that instead of the real problem, which is that the file
            // has no content.
            0 => return Err(
                "exhibit config YAML: no document — the file is empty or contains only comments"
                    .into(),
            ),
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
                "exhibit config YAML: top level is a {}, expected a mapping of key: value pairs",
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
                    "exhibit config YAML: key is a {}, expected a string",
                    yaml_type_name(other)
                ))
            }
        };
        let value = match v {
            Yaml::String(s) => Value::Str(s.clone()),
            // Non-negative only, matching the JSON subset's u64: a negative
            // number is out of range for the format, not just for these keys,
            // so it is refused here in the same words a `.json` file's `-1`
            // gets.
            Yaml::Integer(i) if *i >= 0 => Value::Num(*i as u64),
            Yaml::Integer(i) => {
                return Err(format!(
                    "exhibit config YAML: key {key:?} has negative value {i} — this format \
                     carries strings and non-negative integers only"
                ))
            }
            Yaml::Boolean(_) => {
                return Err(format!(
                    "exhibit config YAML: key {key:?} resolved to a boolean — YAML reads bare \
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

/// Refuse a document that declares an anchor (`&name`) or uses an alias
/// (`*name`), before it is loaded.
///
/// The check has to run at the event level: `YamlLoader` resolves `*name` into
/// a copy of the anchored node, so by the time it returns a tree the alias is
/// indistinguishable from a spelled-out value and [`Yaml::Alias`] never appears
/// in a loaded document. An anchor with no alias is refused as well, so the
/// refusal does not depend on how far the operator got.
///
/// A config that means something other than what it says is the failure this
/// subset exists to prevent, and alias expansion is also the one unbounded
/// allocation on the startup path.
/// See docs/design/exhibit-config.md#file-format.
fn refuse_anchors_and_aliases(text: &str) -> Result<(), String> {
    let mut parser = yaml_rust2::parser::Parser::new_from_str(text);
    loop {
        // A syntax error belongs to YamlLoader: return Ok and let it produce
        // the marked parse error a line later, so the operator gets one
        // message rather than two partial ones.
        let Ok((event, _marker)) = parser.next_token() else {
            return Ok(());
        };
        let anchor_id =
            match &event {
                Event::Alias(_) => return Err(
                    "exhibit config YAML: uses an alias (`*name`), which this format refuses. An \
                     alias makes the file mean something it does not say -- the loader replaces \
                     it with a copy of the anchored value -- and it has no JSON equivalent, so \
                     the same config could not be written in the .json form. Write the value out."
                        .into(),
                ),
                Event::Scalar(_, _, id, _) => *id,
                Event::MappingStart(id, _) | Event::SequenceStart(id, _) => *id,
                Event::StreamEnd => return Ok(()),
                _ => 0,
            };
        // Anchor ids start at 1; 0 means "no anchor".
        if anchor_id > 0 {
            return Err(
                "exhibit config YAML: declares an anchor (`&name`), which this format refuses. \
                 Anchors exist to be referenced by aliases, which make a file mean something it \
                 does not say and have no JSON equivalent. Write the value out."
                    .into(),
            );
        }
    }
}

/// Name a `Yaml` node's kind for an error message, in the words an operator
/// editing YAML would recognise.
///
/// Returns the noun without an article, so each call site supplies its own
/// ("top level is a list", "has a nested mapping value"). Adding an article
/// here would double it at the call sites that already write one.
fn yaml_type_name(y: &Yaml) -> &'static str {
    match y {
        Yaml::Real(_) => "decimal number (this format takes integers only)",
        Yaml::Integer(_) => "integer",
        Yaml::String(_) => "string",
        Yaml::Boolean(_) => "boolean",
        Yaml::Array(_) => "list",
        Yaml::Hash(_) => "nested mapping",
        // Unreachable in a loaded document: refuse_anchors_and_aliases rejects
        // both halves of the feature before the load. Kept so the match stays
        // exhaustive without a catch-all that would absorb a future variant.
        Yaml::Alias(_) => "alias (`*anchor`)",
        Yaml::Null => "null (an empty value)",
        Yaml::BadValue => "unreadable value",
    }
}

/// Does `p` exist, distinguishing "not there" from "cannot tell"?
///
/// `Path::exists()` would be one line, but it maps every error to `false`,
/// including a permission error on a parent directory. That reports an
/// unreadable config as a missing one and tells the operator to create a file
/// they can see. See docs/design/startup-checks.md#message-rules.
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
/// came from. `Ok(None)` means no config exists.
///
/// The one function in this module that touches the filesystem, because which
/// file is the config is a policy that dexd and `dex-exhibit-apply` must answer
/// identically: two copies of it would drift, and the apply tool would then
/// reconcile the kernel cmdline against a file the player does not read.
/// `defaults` is injectable, so the policy stays testable against a temp
/// directory.
///
/// `Ok(None)` is not an error: [`resolve_display`] states the "no exhibit
/// config" refusal, in one place. Every other failure is stated here, because
/// each is a distinct operational fact with a distinct repair.
/// See docs/design/exhibit-config.md#config-location.
pub fn load_exhibit_config(
    override_path: Option<&str>,
    defaults: &[&str],
) -> Result<Option<(ExhibitConfig, String)>, String> {
    let path = match override_path {
        // A named file that is not there is a different fact from "no config
        // was ever created", and must not borrow that message: "create
        // /opt/dex/exhibit.yaml" is wrong advice for an operator who named a
        // different path.
        Some(p) => {
            if !config_exists(p)? {
                return Err(format!(
                    "--exhibit-config {p:?}: no such file. (This is the explicitly named path; \
                     drop --exhibit-config to look in the assets directory instead.)"
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
    /// The video this exhibit plays. Optional in the file, but something must
    /// name it — see [`resolve_asset`].
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
    /// parser. An unparseable config and a missing one lead to the same place:
    /// dexd refuses to start rather than run on a display nobody stated.
    ///
    /// Callers get `format` from [`ConfigFormat::from_path`], never from the
    /// bytes — see that type for why.
    pub fn parse(text: &str, format: ConfigFormat) -> Result<ExhibitConfig, String> {
        match format {
            ConfigFormat::Json => Self::from_json(text),
            ConfigFormat::Yaml => Self::from_yaml(text),
        }
    }

    /// Parse and validate an exhibit config's strict JSON text.
    pub fn from_json(text: &str) -> Result<ExhibitConfig, String> {
        Self::from_pairs(parse_flat_json(text).map_err(|e| format!("exhibit config JSON: {e}"))?)
    }

    /// Parse and validate an exhibit config's YAML text.
    pub fn from_yaml(text: &str) -> Result<ExhibitConfig, String> {
        Self::from_pairs(parse_flat_yaml(text)?)
    }

    /// Map a parsed flat key/value list onto the struct, and validate it.
    ///
    /// This is the whole schema, and both formats reach it unchanged: every key
    /// name, default, grammar check and message lives here once, so a `.json`
    /// and a `.yaml` file expressing the same config cannot differ in meaning
    /// or in what they refuse. Unknown keys are refused here, which is where
    /// this parser's contract departs from `Sidecar::from_json`'s.
    /// See docs/design/exhibit-config.md#config-keys.
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
                        "exhibit config: unknown key {other:?} -- known keys: asset, \
                         display_mode, kms_force, connector, display, venue, note. A \
                         misspelled key would otherwise be ignored, dropping the setting it \
                         was meant to make"
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
                    "exhibit config: invalid asset {a:?} (expect the path to the file to \
                     play, e.g. \"loop.265\" beside this config or \"/opt/dex/loop.265\" -- \
                     no trailing slash, no control characters)"
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

/// Where a bound display config came from: the exhibit config, or the test-rig
/// flags (`--test-rig-no-sidecar` with an optional `--mode <M>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySource {
    /// The exhibit config — how a deployed player states its display.
    Config,
    /// The test-rig flags (`--test-rig-no-sidecar` with an optional
    /// `--mode <M>`). Never how a show runs.
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
/// | config | `--mode` | test rig | result |
/// |---|---|---|---|
/// | present | absent | no | config binds |
/// | present | == config | no | config binds (cross-check) |
/// | present | != config | no | refuse, naming both |
/// | absent | any | no | refuse: no exhibit config |
/// | any | any | yes | the command line binds (`--mode`, or `"auto"`) |
///
/// `--test-rig-no-sidecar` (test rig only) also skips the cmdline check in
/// main.rs, since a test rig runs on hand-managed boot state. That is why this
/// function reports `kms_force: "none"` and the default connector on that
/// branch instead of anything taken from a config it does not consult.
/// See docs/design/exhibit-config.md#display-mode.
pub fn resolve_display(
    config: Option<&ExhibitConfig>,
    cli_mode: Option<&str>,
    test_rig_no_sidecar: bool,
) -> Result<ResolvedDisplay, String> {
    if test_rig_no_sidecar {
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
        None => Err(format!(
            "no exhibit config: dexd refuses to guess the display. The package installs none. \
             Create {DEFAULT_EXHIBIT_CONFIG_PATH} next to the video, with at least these two \
             lines:\n\n    asset: loop.265\n    display_mode: auto\n\nOr pass \
             --test-rig-no-sidecar on a test rig."
        )),
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
                 exhibit config is authoritative) or fix {DEFAULT_EXHIBIT_CONFIG_PATH}",
                cfg.display_mode
            )),
        },
    }
}

/// Where the asset path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSource {
    /// The exhibit config's `asset` key — how a deployed player names its
    /// video.
    Config,
    /// A path given on the command line: a test rig, or a one-off on a device
    /// without editing the exhibit config. Never how a show runs.
    Cli,
}

/// The resolved asset: which file to play, and who said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub path: String,
    pub source: AssetSource,
}

/// Decide which video to play. The table mirrors [`resolve_display`] and
/// `sidecar::resolve_fps`:
///
/// | config `asset` | command-line path | test rig | result |
/// |---|---|---|---|
/// | present | absent | no | config binds |
/// | present | == config | no | config binds (cross-check) |
/// | present | != config | no | refuse, naming both |
/// | absent | present | no | the command line binds, logged as such |
/// | absent | absent | no | refuse: nothing names a video |
/// | any | present | yes | the command line binds; config ignored |
/// | any | absent | yes | refuse: the test rig must be explicit |
///
/// Nothing defaults to `/opt/dex/loop.265`: a mistyped `asset` key would
/// otherwise play last season's video with every metric healthy, so the row
/// where nothing names a video refuses and prints the line to add. The
/// config's `asset` is resolved against the config file's directory first, so
/// a bare `loop.265` beside `/opt/dex/exhibit.yaml` and `/opt/dex/loop.265` on
/// the command line are one file to the cross-check.
/// See docs/design/exhibit-config.md#asset-resolution.
pub fn resolve_asset(
    config: Option<&ExhibitConfig>,
    config_dir: &str,
    cli_path: Option<&str>,
    test_rig_no_sidecar: bool,
) -> Result<ResolvedAsset, String> {
    if test_rig_no_sidecar {
        return match cli_path {
            Some(p) => Ok(ResolvedAsset {
                path: p.to_string(),
                source: AssetSource::Cli,
            }),
            None => Err(
                "--test-rig-no-sidecar consults no exhibit config, so the asset must be given on \
                 the command line: dexd <stream.265> --test-rig-no-sidecar --fps <F>"
                    .into(),
            ),
        };
    }
    // The config's asset is resolved before the cross-check, so a relative
    // `asset:` and the absolute path a technician types on the command line
    // are compared as one file rather than as two strings.
    let config_asset = config
        .and_then(|c| c.asset.as_deref())
        .map(|a| asset_in_config_dir(config_dir, a));
    match (config_asset.as_deref(), cli_path) {
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
             line. Add it to the exhibit config -- `asset: loop.265` (YAML) or \
             `\"asset\": \"loop.265\"` (JSON), a file next to the config or an absolute path -- \
             which is what lets several videos sit in /opt/dex with the config choosing one"
                .into(),
        ),
    }
}

/// Find the `video=<connector>:<mode>` token that names `connector`, inside a
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

/// Find a connectorless `video=` token (`video=WxH@R`, with no `conn:` prefix).
///
/// The kernel's grammar accepts this shape too, and it forces every connector.
/// Neither [`check_cmdline_matches`] nor [`reconcile_cmdline`] can reason about
/// one — which connector does it bind, and how would the kernel arbitrate it
/// against a per-connector token? — so both refuse when one is present.
/// Returns the whole token, for the refusal message to name.
pub fn connectorless_video_token(cmdline: &str) -> Option<String> {
    cmdline.split_whitespace().find_map(|tok| {
        let rest = tok.strip_prefix("video=")?;
        (!rest.contains(':')).then(|| tok.to_string())
    })
}

/// Check that the exhibit config and the kernel cmdline's `video=` token agree.
///
/// `kms_force == "none"` means no token is expected for this connector;
/// anything else means this exact token is expected. An operator who edits one
/// and forgets the other is refused at the next start, instead of the display
/// running on whatever mode the kernel booted with.
///
/// Every refusal names both repairs, because this check cannot tell which side
/// is stale: the cmdline may be a leftover, or it may carry a force the venue
/// needs that a freshly written config does not know about yet. Naming only
/// `dex-exhibit-apply` in the second case would tell the operator to remove a
/// force the display needs, after which `display_mode=auto` plays at whatever
/// the connector negotiates.
/// See docs/design/exhibit-config.md#kernel-cmdline.
pub fn check_cmdline_matches(
    cmdline: &str,
    connector: &str,
    kms_force: &str,
) -> Result<(), String> {
    if let Some(tok) = connectorless_video_token(cmdline) {
        return Err(format!(
            "the kernel cmdline carries a connectorless token {tok:?}, which forces all \
             connectors -- this check cannot reconcile it with the exhibit config's \
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
             carries video={connector}:{found} -- either the config is stale (a force this \
             venue needs, e.g. a display that builds no 4K mode unforced: set \
             kms_force={found:?} and a matching display_mode in the exhibit config to \
             keep it) or the cmdline is (run 'sudo dex-exhibit-apply' and reboot to remove \
             the force)"
        )),
        (want, Some(found)) if want == found => Ok(()),
        (want, Some(found)) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries \
             video={connector}:{found} -- either the config is stale (fix kms_force in \
             the exhibit config to match the venue) or the cmdline is (run 'sudo \
             dex-exhibit-apply' and reboot)"
        )),
        (want, None) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries no video= \
             token for {connector} -- either the config is stale (set kms_force=none in \
             the exhibit config if this venue needs no force) or the cmdline is (run \
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

/// Does `modes_text` — the verbatim contents of
/// `/sys/class/drm/card*-<connector>/modes`, one `WxH` per line with no refresh
/// column — list `want_wh`? Pure over fixture text, so a resolution the
/// connected display cannot show is testable without real DRM.
pub fn sysfs_modes_contains(modes_text: &str, want_wh: &str) -> bool {
    modes_text.lines().map(str::trim).any(|l| l == want_wh)
}

/// Rewrite a single-line `cmdline.txt` so its `video=<connector>:...` token
/// matches `kms_force`, leaving every other token, its position and every other
/// connector's `video=` token untouched. `kms_force == "none"` removes the
/// token for this connector. Idempotent: an existing token is replaced in
/// place, so a second run finds it already correct and changes nothing.
/// See docs/design/exhibit-config.md#kernel-cmdline.
pub fn reconcile_cmdline(
    cmdline_text: &str,
    connector: &str,
    kms_force: &str,
) -> Result<String, String> {
    let trimmed = cmdline_text.trim_end_matches(['\n', '\r']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err("cmdline.txt must be a single line".into());
    }
    if !is_valid_kms_force(kms_force) {
        return Err(format!(
            "reconcile_cmdline: invalid kms_force {kms_force:?}"
        ));
    }
    // A connectorless `video=` token forces every connector; rewriting around
    // it would leave two overlapping forces for the kernel to arbitrate. See
    // connectorless_video_token.
    if let Some(tok) = connectorless_video_token(trimmed) {
        return Err(format!(
            "cmdline.txt carries a connectorless token {tok:?}, which forces all connectors \
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
            // Leading zeros are accepted: positive_int asks only for digits
            // with at least one nonzero, the same rule sidecar::is_valid_fps
            // applies to its numerator and denominator.
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
            // A rational refresh fails mpv's option parser, which leaves the
            // service restarting; a decimal one plays the rounded integer
            // mode. See is_valid_display_mode.
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
            "auto",            // auto is a display_mode concept, not kms_force
            "3840x2160@29.97", // fractional refresh: kernel video= grammar has none
            "3840x2160",
            "3840x2160@",
            "3840x2160@30d", // lowercase d is not the force suffix
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
                        "connector":"HDMI-A-1","display":"gallery panel",
                        "venue":"east wall","note":"forced mode, see the config"}"#;
        let c = ExhibitConfig::from_json(text).unwrap();
        assert_eq!(c.display_mode, "3840x2160@30");
        assert_eq!(c.kms_force, "3840x2160@30");
        assert_eq!(c.connector, "HDMI-A-1");
        assert_eq!(c.display.as_deref(), Some("gallery panel"));
        assert_eq!(c.venue.as_deref(), Some("east wall"));
        assert_eq!(c.note.as_deref(), Some("forced mode, see the config"));
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
    fn unknown_keys_are_refused() {
        let e =
            ExhibitConfig::from_json(r#"{"display_mode":"auto","future_key":"x"}"#).unwrap_err();
        assert!(e.contains("unknown key") && e.contains("future_key"), "{e}");
    }

    #[test]
    fn a_typo_in_kms_force_is_refused_as_an_unknown_key() {
        // The reason unknown keys are refused: "kms_forse" must be a parse
        // error, not an ignored key that leaves the force unset.
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
        assert!(ExhibitConfig::from_json(r#"{"display_mode":"auto","kms_force":"nope"}"#).is_err());
        assert!(ExhibitConfig::from_json(r#"{"display_mode":"auto","connector":"DP-1"}"#).is_err());
    }

    #[test]
    fn display_mode_as_number_is_refused() {
        let e = ExhibitConfig::from_json(r#"{"display_mode":30}"#).unwrap_err();
        assert!(e.contains("string"), "{e}");
    }

    #[test]
    fn duplicate_keys_are_refused_inherited_from_the_sidecar_grammar() {
        let e =
            ExhibitConfig::from_json(r#"{"display_mode":"auto","display_mode":"3840x2160@30"}"#)
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
    fn display_comes_from_the_exhibit_config() {
        let c = cfg("3840x2160@30", "3840x2160@30");
        let r = resolve_display(Some(&c), None, false).unwrap();
        assert_eq!(r.display_mode, "3840x2160@30");
        assert_eq!(r.kms_force, "3840x2160@30");
        assert_eq!(r.source, DisplaySource::Config);
    }

    #[test]
    fn an_agreeing_mode_is_allowed_and_a_disagreeing_one_is_refused() {
        let c = cfg("3840x2160@30", "none");
        assert!(resolve_display(Some(&c), Some("3840x2160@30"), false).is_ok());
        let e = resolve_display(Some(&c), Some("2560x1440@60"), false).unwrap_err();
        assert!(
            e.contains("3840x2160@30") && e.contains("2560x1440@60"),
            "{e}"
        );
    }

    /// The refusal names the file to create -- in the assets directory,
    /// beside the video -- and shows the two lines that file needs.
    #[test]
    fn a_missing_exhibit_config_is_refused() {
        let e = resolve_display(None, Some("3840x2160@30"), false).unwrap_err();
        assert!(e.contains("exhibit config"), "{e}");
        assert!(
            e.contains("/opt/dex/exhibit.yaml"),
            "the refusal must name the file to create: {e}"
        );
        assert!(
            e.contains("asset:") && e.contains("display_mode:"),
            "the refusal must show a minimal example: {e}"
        );
        assert!(resolve_display(None, None, false).is_err());
    }

    #[test]
    fn the_test_rig_flag_ignores_the_exhibit_config() {
        let c = cfg("2560x1440@60", "2560x1440@60");
        // --mode on the command line wins even against the config.
        let r = resolve_display(Some(&c), Some("3840x2160@30"), true).unwrap();
        assert_eq!(r.display_mode, "3840x2160@30");
        assert_eq!(r.source, DisplaySource::Bench);
        assert_eq!(r.kms_force, DEFAULT_KMS_FORCE);
    }

    #[test]
    fn on_a_test_rig_a_missing_mode_means_auto() {
        // resolve_fps requires --fps under the same flag; resolve_display
        // takes a missing --mode as "auto". See its table.
        let r = resolve_display(None, None, true).unwrap();
        assert_eq!(r.display_mode, "auto");
        assert_eq!(r.source, DisplaySource::Bench);
    }

    #[test]
    fn an_invalid_mode_on_a_test_rig_is_refused() {
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
    fn cmdline_check_passes_when_no_token_is_expected_and_none_is_present() {
        assert!(check_cmdline_matches("console=ttyS0 quiet", "HDMI-A-1", "none").is_ok());
    }

    #[test]
    fn cmdline_check_refuses_an_unexpected_token_and_names_the_fix() {
        let e =
            check_cmdline_matches("video=HDMI-A-1:3840x2160@30", "HDMI-A-1", "none").unwrap_err();
        assert!(
            e.contains("dex-exhibit-apply") && e.contains("3840x2160@30"),
            "{e}"
        );
    }

    #[test]
    fn cmdline_check_refuses_a_wrong_mode_naming_both() {
        let e = check_cmdline_matches("video=HDMI-A-1:3840x2160@30", "HDMI-A-1", "2560x1440@60")
            .unwrap_err();
        assert!(
            e.contains("2560x1440@60") && e.contains("3840x2160@30"),
            "{e}"
        );
    }

    #[test]
    fn cmdline_check_treats_another_connectors_token_as_absent() {
        // The configured connector has no token, though another connector
        // does: the check must refuse, naming the missing token.
        let e = check_cmdline_matches("video=HDMI-A-2:1920x1080@60", "HDMI-A-1", "3840x2160@30")
            .unwrap_err();
        assert!(e.contains("no video=") || e.contains("carries no"), "{e}");
    }

    #[test]
    fn cmdline_check_refuses_a_connectorless_video_token() {
        // The kernel grammar also accepts `video=WxH@R` with no connector,
        // which forces every connector; the check refuses and names it.
        let e = check_cmdline_matches(
            "console=ttyS0 video=1920x1080@60 quiet",
            "HDMI-A-1",
            "3840x2160@30",
        )
        .unwrap_err();
        assert!(
            e.contains("video=1920x1080@60") && e.contains("all connectors"),
            "{e}"
        );
        // Also when the per-connector expectation is "none": the global
        // force still binds this connector, so a pass would be wrong.
        let e = check_cmdline_matches("video=1920x1080@60", "HDMI-A-1", "none").unwrap_err();
        assert!(e.contains("connectorless"), "{e}");
    }

    #[test]
    fn connectorless_video_token_ignores_per_connector_tokens() {
        assert_eq!(
            connectorless_video_token("video=HDMI-A-1:3840x2160@30 quiet"),
            None
        );
        assert_eq!(
            connectorless_video_token("quiet video=1024x768"),
            Some("video=1024x768".to_string())
        );
        assert_eq!(connectorless_video_token("console=ttyS0 quiet"), None);
    }

    #[test]
    fn cmdline_check_passes_when_the_expected_token_is_present() {
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
        let once =
            reconcile_cmdline("console=ttyS0 rootwait", "HDMI-A-1", "3840x2160@30D").unwrap();
        let twice = reconcile_cmdline(&once, "HDMI-A-1", "3840x2160@30D").unwrap();
        assert_eq!(once, twice);

        // And the "no change" case: apply the force that is already present,
        // which is what dex-exhibit-apply does on a device already set up.
        let already = "console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait";
        let out = reconcile_cmdline(already, "HDMI-A-1", "3840x2160@30").unwrap();
        assert_eq!(out, already);
    }

    #[test]
    fn reconcile_refuses_a_connectorless_video_token() {
        // Appending a per-connector token around a global force would leave
        // the kernel two overlapping forces to arbitrate. Refuse, and name
        // the token.
        let e = reconcile_cmdline(
            "console=ttyS0 video=1024x768 rootwait",
            "HDMI-A-1",
            "3840x2160@30",
        )
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
    // dispatch. The parsers get their own sections below.

    #[test]
    fn extension_decides_the_parser() {
        assert_eq!(
            ConfigFormat::from_path("/opt/dex/exhibit.json").unwrap(),
            ConfigFormat::Json
        );
        assert_eq!(
            ConfigFormat::from_path("/opt/dex/exhibit.yaml").unwrap(),
            ConfigFormat::Yaml
        );
        assert_eq!(
            ConfigFormat::from_path("/opt/dex/exhibit.yml").unwrap(),
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
            "/opt/dex/exhibit",          // no extension at all
            "/opt/dex/exhibit.txt",      // an extension, but not one dexd reads
            "/opt/dex/.json",            // a dotfile named .json: no extension
            "/opt/dex/exhibit.json.bak", // the backup, not the config
        ] {
            let e = ConfigFormat::from_path(p).unwrap_err();
            assert!(e.contains(".json") && e.contains(".yaml"), "{p}: {e}");
        }
    }

    /// What the dispatch is for: YAML syntax inside a file named `.json` is
    /// refused, even though a YAML parser would accept it. A `.json` file that
    /// only `dexd` can read cannot be read by `jq`, or by anything else that
    /// picks its parser from the extension.
    #[test]
    fn yaml_syntax_in_a_json_file_is_refused() {
        let yaml_text = "display_mode: 3840x2160@30\nkms_force: none\n";
        // The YAML parser accepts it, proving the input is valid YAML...
        assert!(ExhibitConfig::parse(yaml_text, ConfigFormat::Yaml).is_ok());
        // ...and the JSON parser must still refuse it under a .json name.
        assert!(ExhibitConfig::parse(yaml_text, ConfigFormat::Json).is_err());
    }

    /// The converse parses: JSON is a subset of YAML, so strict JSON written
    /// into a `.yaml` file is still read, which lets one tool emit one format
    /// for both names.
    #[test]
    fn json_text_parses_under_the_yaml_parser_too() {
        let json_text = r#"{"display_mode":"3840x2160@30","kms_force":"none"}"#;
        let as_json = ExhibitConfig::parse(json_text, ConfigFormat::Json).unwrap();
        let as_yaml = ExhibitConfig::parse(json_text, ConfigFormat::Yaml).unwrap();
        assert_eq!(as_json, as_yaml);
    }

    /// The same config in either format produces the same struct. This is the
    /// test that fails first if the two paths stop sharing `from_pairs`.
    #[test]
    fn both_formats_agree_on_a_full_config() {
        let json = ExhibitConfig::from_json(
            r#"{"display_mode":"3840x2160@30","kms_force":"3840x2160@30D",
                "connector":"HDMI-A-2","display":"gallery panel",
                "venue":"gallery east wall","note":"builds no 4K mode unforced"}"#,
        )
        .unwrap();
        let yaml = ExhibitConfig::from_yaml(
            "# the same thing, with the comments JSON cannot carry\n\
             display_mode: 3840x2160@30\n\
             kms_force: 3840x2160@30D    # trailing D: force `connected`\n\
             connector: HDMI-A-2\n\
             display: gallery panel\n\
             venue: gallery east wall\n\
             note: builds no 4K mode unforced\n",
        )
        .unwrap();
        assert_eq!(json, yaml);
    }

    /// Validation is shared, so a YAML file gets the JSON path's messages,
    /// including the unknown-key refusal for a misspelled `kms_forse`.
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

    /// yaml-rust2's loader errors on a duplicate key instead of taking the
    /// last one. The JSON side needs a hand-written serde visitor for the same
    /// rule, so this is locked in by a test: it is a property of the
    /// dependency and could change on a version bump.
    #[test]
    fn yaml_duplicate_key_refused() {
        let e = ExhibitConfig::from_yaml("display_mode: auto\ndisplay_mode: 3840x2160@30\n")
            .unwrap_err();
        assert!(e.contains("duplicate"), "{e}");
    }

    /// A bare `true`/`false` is a boolean, refused with the quoting fix.
    #[test]
    fn yaml_bare_true_false_are_booleans_and_refused() {
        for text in ["display_mode: true\n", "display_mode: auto\nnote: FALSE\n"] {
            let e = ExhibitConfig::from_yaml(text).unwrap_err();
            assert!(e.contains("resolved to a boolean"), "{text:?}: {e}");
            assert!(e.contains("Quote the value"), "{text:?}: {e}");
        }
    }

    /// yaml-rust2 0.11 resolves close to the YAML 1.2 core schema, where only
    /// true and false are booleans: `no` stays the string `"no"` and is refused
    /// by the kms_force grammar. The assertion reads the message, so a version
    /// that adopted YAML 1.1 resolution — under which `kms_force: no` would
    /// mean false — fails here instead of changing what a deployed config
    /// means.
    #[test]
    fn yaml_bare_no_stays_a_string_under_the_1_2_core_schema() {
        let e = ExhibitConfig::from_yaml("display_mode: auto\nkms_force: no\n").unwrap_err();
        assert!(
            e.contains("invalid kms_force \"no\""),
            "expected the grammar refusal for the string \"no\", not a boolean one: {e}"
        );
        assert!(!e.contains("resolved to a boolean"), "{e}");
        // ...and stating it properly is the fix.
        let ok = ExhibitConfig::from_yaml("display_mode: auto\nkms_force: none\n").unwrap();
        assert_eq!(ok.kms_force, "none");
    }

    /// Where yaml-rust2 departs from the YAML 1.2 core schema: core lists
    /// `null | Null | NULL | ~ | empty` as null, while this library's
    /// `from_str` matches only `""`, `"~"` and `"null"`, case-sensitively, so
    /// the capitalised spellings arrive as ordinary strings. That is harmless
    /// for this schema — only the informational keys could carry such a value
    /// — but it is the kind of near-miss a reader would otherwise rely on.
    #[test]
    fn yaml_null_resolution_is_case_sensitive_unlike_the_1_2_core_schema() {
        for null_spelling in ["null", "~", ""] {
            let e =
                ExhibitConfig::from_yaml(&format!("display_mode: auto\nnote: {null_spelling}\n"))
                    .unwrap_err();
            assert!(e.contains("null"), "{null_spelling:?}: {e}");
        }
        for string_spelling in ["Null", "NULL"] {
            let c =
                ExhibitConfig::from_yaml(&format!("display_mode: auto\nnote: {string_spelling}\n"))
                    .expect("core would call this null; yaml-rust2 does not");
            assert_eq!(c.note.as_deref(), Some(string_spelling));
        }
    }

    /// Scalar resolution happens before the subset check, so it stays
    /// observable, and both formats land in the same place: a bare number
    /// resolves to an integer and is then refused for a string-only key,
    /// as the JSON spelling of the same thing is.
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

    /// Anchors and aliases are refused before the load.
    ///
    /// `YamlLoader` resolves `*name` into a copy of the anchored node, so
    /// `Yaml::Alias` never reaches the node-type check: without the pre-scan
    /// the config below is accepted, with `venue` carrying a value the file
    /// never assigns to it.
    #[test]
    fn yaml_anchors_and_aliases_are_refused_before_the_load() {
        // venue is never assigned in the text.
        let e = ExhibitConfig::from_yaml("display_mode: auto\nnote: &a hello\nvenue: *a\n")
            .unwrap_err();
        assert!(e.contains("anchor") || e.contains("alias"), "{e}");

        // An anchor with no alias is refused as well: accepting it would make
        // the refusal depend on how far the operator got.
        let e = ExhibitConfig::from_yaml("display_mode: auto\nnote: &unused hello\n").unwrap_err();
        assert!(e.contains("anchor"), "{e}");

        // The spelled-out equivalent is accepted, so the refusal costs one
        // retyped value.
        let ok =
            ExhibitConfig::from_yaml("display_mode: auto\nnote: hello\nvenue: hello\n").unwrap();
        assert_eq!(ok.note.as_deref(), Some("hello"));
        assert_eq!(ok.venue.as_deref(), Some("hello"));
    }

    /// A syntax error still produces YamlLoader's own marked message, rather
    /// than a vaguer one from the anchor pre-scan that runs first.
    #[test]
    fn the_anchor_prescan_does_not_swallow_real_syntax_errors() {
        let e = ExhibitConfig::from_yaml("display_mode: auto\nnote:\n\tx: 1\n").unwrap_err();
        assert!(
            e.contains("tab"),
            "expected the scanner's own diagnostic: {e}"
        );
    }

    /// Error messages read as English: `yaml_type_name` returns bare nouns and
    /// each call site supplies its own article, so no message doubles it.
    #[test]
    fn type_names_do_not_double_their_article() {
        for text in [
            "display_mode: auto\nnote:\n  a: b\n",
            "display_mode: auto\nnote:\n  - a\n",
            "- a\n- b\n",
            "just a string\n",
        ] {
            let e = ExhibitConfig::from_yaml(text).unwrap_err();
            assert!(!e.contains(" a a "), "doubled article: {e}");
            assert!(!e.contains(" a an "), "doubled article: {e}");
        }
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
        let e = ExhibitConfig::from_yaml("display_mode: auto\n---\ndisplay_mode: 3840x2160@30\n")
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

    /// An integer value parses, and the shared mapper then refuses it for
    /// these keys in the same words the JSON path uses. A negative integer is
    /// out of range for the format itself.
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
            pick_default_config(&["/opt/dex/exhibit.yaml"]).unwrap(),
            Some("/opt/dex/exhibit.yaml")
        );
        assert_eq!(
            pick_default_config(&["/opt/dex/exhibit.json"]).unwrap(),
            Some("/opt/dex/exhibit.json")
        );
    }

    /// Two configs at once are refused, naming both. Precedence would let an
    /// operator edit one file while the player reads the other.
    #[test]
    fn two_default_configs_refuse_naming_both() {
        let e = pick_default_config(&DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(
            e.contains("/opt/dex/exhibit.yaml") && e.contains("/opt/dex/exhibit.json"),
            "the refusal must name both files in the assets directory: {e}"
        );
        assert!(e.contains("--exhibit-config"), "{e}");
        // The message names the likely cause: writing exhibit.yaml leaves the
        // older exhibit.json beside it, so the operator did one correct thing
        // and still got refused.
        assert!(
            e.contains("sudo rm /opt/dex/exhibit.json"),
            "must name the file to delete as the fix: {e}"
        );
    }

    /// The hint is conditional: two YAML files must not produce advice to
    /// remove a JSON file that has nothing to do with the collision.
    #[test]
    fn the_delete_hint_only_appears_when_a_json_is_involved() {
        let e = pick_default_config(&["/srv/a.yaml", "/srv/b.yml"]).unwrap_err();
        assert!(!e.contains("sudo rm"), "{e}");
    }

    // ---- resolve_asset: the whole decision table -------------------------

    #[test]
    fn the_asset_comes_from_the_exhibit_config() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), "/opt/dex", None, false).unwrap();
        assert_eq!(r.path, "/opt/dex/spring.265");
        assert_eq!(r.source, AssetSource::Config);
    }

    #[test]
    fn an_agreeing_command_line_path_cross_checks_and_the_config_binds() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), "/opt/dex", Some("/opt/dex/spring.265"), false).unwrap();
        assert_eq!(r.source, AssetSource::Config);
    }

    #[test]
    fn a_contradicting_command_line_path_is_refused_naming_both() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let e =
            resolve_asset(Some(&c), "/opt/dex", Some("/opt/dex/autumn.265"), false).unwrap_err();
        assert!(e.contains("spring.265") && e.contains("autumn.265"), "{e}");
    }

    /// A config with no `asset` key still accepts a path given by hand: the
    /// one-off case of trying another file on a deployed device. The result
    /// says the path came from the command line, so the log stays readable.
    #[test]
    fn a_command_line_path_works_when_the_config_names_no_asset() {
        let c = cfg("auto", "none");
        let r = resolve_asset(Some(&c), "/opt/dex", Some("/opt/dex/try.265"), false).unwrap();
        assert_eq!(r.source, AssetSource::Cli);
    }

    /// The row where nothing names a video: refuse, and print the line to
    /// add.
    #[test]
    fn no_asset_named_anywhere_is_refused_without_a_default() {
        let c = cfg("auto", "none");
        let e = resolve_asset(Some(&c), "/opt/dex", None, false).unwrap_err();
        assert!(e.contains("no asset"), "{e}");
        // ...and it names the exact line to add, in both formats.
        assert!(e.contains("asset: loop.265"), "{e}");
        assert!(e.contains(r#""asset": "loop.265""#), "{e}");
        // Also with no config at all: that case refuses earlier, in
        // resolve_display, but this function invents no path either.
        assert!(resolve_asset(None, "/opt/dex", None, false).is_err());
    }

    #[test]
    fn a_test_rig_takes_the_command_line_path_and_ignores_the_config() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let r = resolve_asset(Some(&c), "/opt/dex", Some("/tmp/test-rig.265"), true).unwrap();
        assert_eq!(r.path, "/tmp/test-rig.265");
        assert_eq!(r.source, AssetSource::Cli);
    }

    #[test]
    fn a_test_rig_without_a_path_is_refused() {
        let c = cfg_with_asset("/opt/dex/spring.265");
        let e = resolve_asset(Some(&c), "/opt/dex", None, true).unwrap_err();
        assert!(e.contains("command line"), "{e}");
    }

    // ---- a relative asset resolves against the config's directory --------

    /// A bare file name names the file beside the config — the deployment
    /// shape, where the video, its sidecar and `exhibit.yaml` sit together in
    /// `/opt/dex`.
    #[test]
    fn a_relative_asset_resolves_against_the_config_directory() {
        let c = cfg_with_asset("artwork.265");
        let r = resolve_asset(Some(&c), "/opt/dex", None, false).unwrap();
        assert_eq!(r.path, "/opt/dex/artwork.265");
        assert_eq!(r.source, AssetSource::Config);
    }

    /// A `../` prefix is joined, not rejected and not normalised: the kernel
    /// resolves it at open time, and the printed path shows both halves the
    /// operator wrote.
    #[test]
    fn a_relative_asset_may_leave_the_config_directory() {
        let c = cfg_with_asset("../media/artwork.265");
        let r = resolve_asset(Some(&c), "/opt/dex", None, false).unwrap();
        assert_eq!(r.path, "/opt/dex/../media/artwork.265");
    }

    /// An absolute `asset` is untouched by the directory.
    #[test]
    fn an_absolute_asset_ignores_the_config_directory() {
        let c = cfg_with_asset("/srv/art/artwork.265");
        let r = resolve_asset(Some(&c), "/opt/dex", None, false).unwrap();
        assert_eq!(r.path, "/srv/art/artwork.265");
    }

    /// The cross-check compares the resolved path, so a bare name in the
    /// config and the full path on the command line agree.
    #[test]
    fn the_cross_check_compares_the_resolved_path() {
        let c = cfg_with_asset("artwork.265");
        let r = resolve_asset(Some(&c), "/opt/dex", Some("/opt/dex/artwork.265"), false).unwrap();
        assert_eq!(r.path, "/opt/dex/artwork.265");
        assert_eq!(r.source, AssetSource::Config);
        let e = resolve_asset(Some(&c), "/opt/dex", Some("/opt/dex/other.265"), false).unwrap_err();
        assert!(
            e.contains("/opt/dex/artwork.265") && e.contains("/opt/dex/other.265"),
            "the refusal must name both resolved paths: {e}"
        );
    }

    #[test]
    fn config_dir_is_the_directory_the_config_sits_in() {
        assert_eq!(config_dir("/opt/dex/exhibit.yaml"), "/opt/dex");
        assert_eq!(config_dir("/exhibit.yaml"), "/");
        assert_eq!(config_dir("exhibit.yaml"), ".");
    }

    #[test]
    fn asset_in_config_dir_joins_without_doubling_the_separator() {
        assert_eq!(asset_in_config_dir("/opt/dex", "a.265"), "/opt/dex/a.265");
        assert_eq!(asset_in_config_dir("/", "a.265"), "/a.265");
        assert_eq!(asset_in_config_dir("/opt/dex", "/srv/a.265"), "/srv/a.265");
    }

    /// The resolver against a real directory: write a config and its video
    /// into one temp directory, load the config through the shared loader, and
    /// check that the bare name resolved to the file beside it.
    #[test]
    fn a_relative_asset_resolves_against_a_real_config_directory() {
        let cfg_path = tmp("relative-asset-exhibit.yaml");
        std::fs::write(&cfg_path, "asset: artwork.265\ndisplay_mode: auto\n").unwrap();
        let (c, path) = load_exhibit_config(Some(&cfg_path), &DEFAULT_EXHIBIT_CONFIG_PATHS)
            .unwrap()
            .unwrap();
        let r = resolve_asset(Some(&c), config_dir(&path), None, false).unwrap();
        let dir = config_dir(&cfg_path);
        assert_eq!(r.path, format!("{dir}/artwork.265"));
        let _ = std::fs::remove_file(&cfg_path);
    }

    // ---- the asset grammar ----------------------------------------------

    #[test]
    fn asset_accepts_absolute_and_relative_paths() {
        assert!(is_valid_asset("/opt/dex/loop.265"));
        assert!(is_valid_asset("/srv/art/Karte–Süd.265")); // non-ASCII is fine
        assert!(is_valid_asset("loop.265")); // beside the config
        assert!(is_valid_asset("./loop.265"));
        assert!(is_valid_asset("../media/loop.265"));
        assert!(!is_valid_asset(""));
        assert!(!is_valid_asset("/opt/dex/")); // a directory, not a file
    }

    /// The asset path is printed into the system log at every start, and the
    /// system log is the only diagnostic channel a deployed player has -- a
    /// newline in it would forge a second log line.
    #[test]
    fn asset_with_a_control_character_is_refused() {
        assert!(!is_valid_asset("/opt/dex/loop.265\ndexd: all fine here"));
        assert!(!is_valid_asset("/opt/dex/loop\t.265"));
    }

    #[test]
    fn asset_parses_from_both_formats_and_is_grammar_checked() {
        let j =
            ExhibitConfig::from_json(r#"{"asset":"/opt/dex/spring.265","display_mode":"auto"}"#)
                .unwrap();
        let y =
            ExhibitConfig::from_yaml("asset: /opt/dex/spring.265\ndisplay_mode: auto\n").unwrap();
        assert_eq!(j, y);
        assert_eq!(j.asset.as_deref(), Some("/opt/dex/spring.265"));

        // A bare file name is valid -- it names the file beside the config.
        let rel = ExhibitConfig::from_yaml("asset: loop.265\ndisplay_mode: auto\n").unwrap();
        assert_eq!(rel.asset.as_deref(), Some("loop.265"));
        let e = ExhibitConfig::from_yaml("asset: /opt/dex/\ndisplay_mode: auto\n").unwrap_err();
        assert!(e.contains("invalid asset"), "{e}");
    }

    // ---- load_exhibit_config, against a real temp directory --------------
    //
    // `defaults` is injectable, so the discovery policy both binaries share
    // is testable here, and not only through the command line on a machine
    // that happens to have /opt/dex.

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
        let found =
            load_exhibit_config(None, &[&tmp("load-absent.yaml"), &tmp("load-absent.json")])
                .unwrap();
        assert!(found.is_none(), "{found:?}");
    }

    /// Why both binaries call this: with both names present it refuses
    /// instead of picking, so `dex-exhibit-apply` cannot reconcile the cmdline
    /// against a file `dexd` will not read.
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

    /// A named file that is missing does not borrow the "no exhibit config,
    /// create the default" message: the operator named a different path, so
    /// advice to create /opt/dex/exhibit.yaml would be wrong.
    #[test]
    fn load_names_the_explicit_path_when_it_is_missing() {
        let p = tmp("load-explicitly-absent.json");
        let _ = std::fs::remove_file(&p);
        let e = load_exhibit_config(Some(&p), &DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(e.contains(&p), "{e}");
        assert!(!e.contains("Create /opt/dex"), "{e}");
    }

    /// A config whose name promises neither format is refused at the
    /// dispatch, before any parse is attempted, though its contents would
    /// parse as either.
    #[test]
    fn load_refuses_an_unrecognised_extension_even_with_valid_contents() {
        let p = tmp("load-nameless.conf");
        std::fs::write(&p, r#"{"display_mode":"auto"}"#).unwrap();
        let e = load_exhibit_config(Some(&p), &DEFAULT_EXHIBIT_CONFIG_PATHS).unwrap_err();
        assert!(e.contains("cannot tell the format"), "{e}");
        let _ = std::fs::remove_file(&p);
    }
}
