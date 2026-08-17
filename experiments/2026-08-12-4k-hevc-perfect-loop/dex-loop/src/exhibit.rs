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
//! belongs to an `/etc/dex/exhibit.json` conffile — venue truth, not asset
//! truth — parsed with the F3 sidecar's own hardened, fail-closed
//! subset-JSON grammar (`sidecar::parse_flat_json`).
//!
//! CORRECTION (2026-08-17): an earlier draft of this doc comment claimed "the
//! M5 soak played a 2160p30 asset on a 1440p Dell" as field evidence that one
//! asset runs on several panels. That run never happened — the M5 soak ran
//! 4K30 on the Cam Link with the Dell disconnected. Removed rather than left
//! to mislead a future reader; the Cam-Link-vs-Dell force disagreement above
//! is real, bench-verified evidence and stands on its own.
//!
//! Format:
//!   {"display_mode":"3840x2160@30","kms_force":"3840x2160@30",
//!    "connector":"HDMI-A-1","display":"...","venue":"...","note":"..."}
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

use crate::sidecar::{is_valid_fps, parse_flat_json, Value};

/// Default path for the exhibit config, overridable with `--exhibit-config`
/// for tests and, in principle, an unusual install layout.
pub const DEFAULT_EXHIBIT_CONFIG_PATH: &str = "/etc/dex/exhibit.json";
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

/// `display_mode` grammar: `"auto"`, or `"WxH@R"` with W, H positive
/// integers and R passing the SAME grammar `--fps`/the sidecar use
/// (`is_valid_fps`) — so `@29.97` and `@30000/1001` are representable,
/// because mpv's `--drm-mode` accepts a fractional refresh and an exhibit
/// states its cadence exactly, the same way the asset does. The `@R` part is
/// mandatory: `"3840x2160"` alone would let mpv pick between 30.00 and 29.97
/// by list order, which is exactly the silent-wrongness class this crate
/// exists to close.
pub fn is_valid_display_mode(s: &str) -> bool {
    if s == "auto" {
        return true;
    }
    match split_mode(s) {
        Some((w, h, r)) => positive_int(w) && positive_int(h) && is_valid_fps(r),
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

/// A parsed, validated exhibit config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExhibitConfig {
    pub display_mode: String,
    pub kms_force: String,
    pub connector: String,
    pub display: Option<String>,
    pub venue: Option<String>,
    pub note: Option<String>,
}

impl ExhibitConfig {
    /// Parse and validate an exhibit config's JSON text. Fail-closed, same
    /// theory as the F3 sidecar: an unparseable exhibit config and a missing
    /// one are the same operational fact — both refuse rather than run on a
    /// display nobody stated.
    ///
    /// UNLIKE the sidecar, unknown keys are refused (module docs above) —
    /// this is the one place this parser's contract deliberately differs
    /// from `Sidecar::from_json`'s.
    pub fn from_json(text: &str) -> Result<ExhibitConfig, String> {
        let kv = parse_flat_json(text).map_err(|e| format!("exhibit config JSON: {e}"))?;
        let mut display_mode = None;
        let mut kms_force = None;
        let mut connector = None;
        let mut display = None;
        let mut venue = None;
        let mut note = None;
        for (k, v) in kv {
            match (k.as_str(), v) {
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
                         sidecar — known keys: display_mode, kms_force, connector, display, \
                         venue, note; a typo here must not silently drop a force)"
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
        Ok(ExhibitConfig {
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

/// The gate that keeps the exhibit config and the KMS-layer `video=` token
/// honest with each other (§3.3 of the design): an operator who edits one and
/// forgets the other must be refused, loudly, naming what to run — not
/// black-screen the gallery on whichever value the kernel happens to have
/// booted with. `kms_force == "none"` means "expect NO token for this
/// connector"; anything else means "expect this EXACT token".
pub fn check_cmdline_matches(cmdline: &str, connector: &str, kms_force: &str) -> Result<(), String> {
    let found = cmdline_video_token(cmdline, connector);
    match (kms_force, found) {
        (DEFAULT_KMS_FORCE, None) => Ok(()),
        (DEFAULT_KMS_FORCE, Some(found)) => Err(format!(
            "exhibit config says kms_force=none for {connector}, but the kernel cmdline \
             carries video={connector}:{found} -- run 'sudo dex-exhibit-apply' and reboot"
        )),
        (want, Some(found)) if want == found => Ok(()),
        (want, Some(found)) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries \
             video={connector}:{found} -- run 'sudo dex-exhibit-apply' and reboot"
        )),
        (want, None) => Err(format!(
            "exhibit config says kms_force={want}, but the kernel cmdline carries no video= \
             token for {connector} -- run 'sudo dex-exhibit-apply' and reboot"
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
            "2560x1440@59.95",
            "3840x2160@30000/1001",
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
            display_mode: display_mode.to_string(),
            kms_force: kms_force.to_string(),
            connector: DEFAULT_CONNECTOR.to_string(),
            display: None,
            venue: None,
            note: None,
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
}
