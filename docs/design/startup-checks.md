# Startup checks

This page lists everything dexd verifies before it hands the video to mpv, in the order it runs them, with what makes each one refuse. It is for a developer reading the source or diagnosing a player that will not start. The same messages with their fixes, for a technician, are in [../guides/reference.md](../guides/reference.md).

## Fail-closed startup

dexd refuses to start when an input is missing or ambiguous, rather than substituting a default. If dexd substituted a default, the player would run with the wrong picture and every metric normal: a guessed frame rate, display mode, or asset plays that way for the length of the exhibition.

No operator is present at an unattended installation to notice a wrong input later, so dexd refuses it at startup (decided).

Any path that degrades without reporting it counts as a bug even when playback works:

- an idle hang
- a fallback to software decoding
- a discarded log
- a wrong `--fps`

Every startup check that refuses exits 2 with a message naming the repair.

## Order of checks

| Check | Refuses when | Skipped by `--test-rig-no-sidecar` |
|---|---|---|
| Argument shape | a flag's value is missing or does not parse; a token is unrecognised | no |
| Test-rig flag pairing | a test-rig timer flag is given without `--test-rig-no-sidecar` | — |
| Exhibit config | none exists; two exist at once; a path given with `--exhibit-config` is absent; the file cannot be read or parsed; a key or value type is outside the schema | yes |
| Display mode | `--mode` contradicts the config's `display_mode` | yes |
| Cmdline check | the config's `kms_force` disagrees with the kernel's `video=` entry for the connector, or that entry names no connector | yes |
| Mode pre-flight | the resolution is not among the modes the connector lists | no |
| Asset resolution | no asset is named anywhere; a command-line path contradicts the config | the path must come from the command line |
| Asset read | the file is unreadable or empty | no |
| Sidecar | it is absent, or outside the strict JSON subset | yes |
| Frame rate | `--fps` contradicts the sidecar | `--fps` becomes required |
| Checksum | the asset's SHA-256 does not match the sidecar's | yes |
| Leading NAL units | the stream does not begin with VPS, SPS, and PPS followed by an IDR | no |
| mpv options | libmpv rejects an option name or value | no |

The display checks run before the asset is read; they are the cheapest in the program. dexd therefore reports a resolution the connector does not list even when the asset path is also wrong.

`--test-rig-no-sidecar` skips everything that reads deployment state: the exhibit config and the `--mode` cross-check against it, the kernel command line, and the sidecar with the frame-rate and checksum checks that depend on it. The mode pre-flight reads the connected hardware, so it runs either way.

## Argument shape

`ExecStart=/usr/bin/dexd` passes no arguments, so dexd has no argument-shape guard for an empty command line: a bare `dexd` proceeds to the exhibit config, which names the asset, and refuses there if the config does not name one.

dexd takes the first token that does not start with `-` as an asset path, which must then agree with the config's resolved path (see [exhibit-config.md](exhibit-config.md#asset-resolution)). Any other unrecognised token prints usage and exits 2.

A flag whose value is missing or malformed also prints usage. Read as absent instead, a dropped `--fps` would skip the sidecar cross-check and a dropped `--mode` would fall back to the mode the connector prefers, both without an error.

## Resolution and refresh

The mode pre-flight validates the resolution half of `display_mode`. The kernel lists one `WxH` per line in `/sys/class/drm/card*-<connector>/modes` and gives no refresh column, so dexd checks the `@R` half against the grammar alone. mpv settles the refresh at video-output init and reports `Could not find mode matching 3840x2160@60` for one the connector does not offer; the symptom and its fix are in [../guides/run-check-troubleshoot.md](../guides/run-check-troubleshoot.md). dexd skips the pre-flight for `display_mode: auto`, which names no resolution.

Reading the modes file needs no privilege and no libmpv, so the check can run this early. The grammar and the reconciliation of the forced display mode with the kernel `video=` token are in [exhibit-config.md](exhibit-config.md).

## Exit codes

| Code | Meaning |
|---|---|
| 2 | Refused before playback. Fix the invocation, config, asset, or sidecar and redeploy; a restart cannot help. |
| 1 | Playback or runtime failure. The service manager restarts the process. |
| 0 | Never, in normal operation: the player is designed not to end. |

dexd exits 2 when libmpv rejects an option: the same asset and flags fail identically on every restart.

More than one site returns exit 1. The stderr line names which failure fired, and dexd prints `playback ended` when the stream ends. Runtime behaviour is in [failure-handling.md](failure-handling.md).

## Startup lines

dexd prints its version and commit to stderr before it parses any argument, so a refused start still names the build that refused.

dexd prints four lines before it hands the video to mpv, shown here for the 3 s 4K test video (14.8 MB, see [measurements.md](measurements.md)):

```
dexd 0.1.0 (<commit>)
dexd: display auto (exhibit config /opt/dex/exhibit.yaml), connector HDMI-A-1, kms-force none
dexd: asset /opt/dex/artwork.265 (exhibit config /opt/dex/exhibit.yaml)
dexd: 14800000 bytes, fps 30 (sidecar), looping endlessly
```

Each line names where its value came from: `exhibit config <path>`, `sidecar`, `command line, not the exhibit config` for an asset given on the command line, or `test rig override, unbound` under `--test-rig-no-sidecar`, where the kms-force field also reads `not checked (test rig)`. With no config found, the display line names the path dexd's refusal asks the operator to create.

## Message rules

### Both repairs

The cmdline check compares the exhibit config with the running kernel's command line, and either can be the outdated one. On a device whose command line carries a force the venue needs, a message prescribing `dex-exhibit-apply` alone would tell the operator to delete that force, after which `display_mode: auto` plays at whatever the connector negotiates. dexd states both values and both repairs: update the config, or run `dex-exhibit-apply` and reboot.

### Distinct messages

A config that is absent, one that is present but unreadable, and one whose path cannot be read at all (`cannot stat`) have three different repairs, so dexd reports them separately. A named `--exhibit-config` file that does not exist gets its own message, pointing back at the assets directory `/opt/dex`: the advice to create `/opt/dex/exhibit.yaml` would be wrong for someone who has just named another path.

### The line to add

When no config names an asset, dexd prints the key in both formats, `asset: artwork.265` for YAML and `"asset": "artwork.265"` for JSON. The package installs no config, and its postinst prints the minimal file to create, so the operator sees the missing config during the install rather than at the next power cycle.

## Test-rig-only override

`--test-rig-no-sidecar` runs dexd on a test rig with no deployment state: it consults no exhibit config and no sidecar, and takes `--fps` and `--mode` as given.

- It requires `--fps`, and exits 2 without one: a deployed player would otherwise take its frame rate from the command line whenever the sidecar was absent, with nothing tying that rate to the video.
- `--mode` is optional and defaults to `auto`.
- The asset must come from the command line, and a sidecar that happens to be present is ignored.

Two flags force a failure on purpose: `--test-rig-force-recovery-after-secs` triggers an in-place recovery on a timer, and `--test-rig-hang-after-secs` hangs the supervisor thread. dexd refuses both with exit 2 unless `--test-rig-no-sidecar` is present, deciding on the command line's shape before reading the asset. Each prints a warning naming itself when armed. The packaged unit passes neither, so neither can be enabled against a deployed asset.

## Refusal output

Every refusal reaches the system log and nothing else: the unit conflicts the console getty away so dexd can take DRM master, and systemd routes stderr there. At the venue every failure therefore shows as a dark screen with no message. The triage command is `journalctl -u dexd -n 20`.

dexd does not write the last refusal to tty1. The unit's stop handler could; that is decided and not built, pending a check of which process owns the text console on the target hardware.

## Test scope

The decision tables behind these checks — the display-mode grammar, display, asset, and frame-rate resolution, the cmdline comparator and the pre-flight parser — are pure functions, exhaustively unit-tested on a workstation. The tests that spawn the binary enforce what those cannot: check order, flag parsing, and the refusal messages. One case stays in the unit tests, a host with no config anywhere: at the binary level its outcome depends on the assets directory of whichever machine runs the suite. See [development.md](development.md).

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| Guess the frame rate when the sidecar is absent | Plays at the wrong speed indefinitely, with no error | The failure is undetectable from inside the running system |
| Default the asset to a fixed path | Plays the previous installation's video after a mistyped config key | Widest consequence of any guess here |
| Resolve two exhibit configs by precedence | Runs yesterday's display mode without saying so | Only the operator knows which file is meant; deleting one is one command |

## Open questions

dexd skips the mode pre-flight for `display_mode: auto`, so a display that, unforced, cannot offer the asset's resolution plays at whatever it negotiates. In that case dexd compares the sidecar's stored width and height against the connector's mode list and warns without refusing. Whether it should refuse, and whether `auto` should remain the default at all, are undecided.
