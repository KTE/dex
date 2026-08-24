# Exhibit config

The exhibit config is the file that says which video plays and how the display is driven. This page covers where it lives, its two formats and shared schema, the grammar behind each key, and how dexd and dex-exhibit-apply(1) keep it in agreement with the kernel command line. It is written for a developer reading `exhibit.rs` or diagnosing a player that will not start; the technician's version is [Configure the exhibit](../guides/configure-exhibit.md).

## Config scope

The display mode is a property of the installation, not of the video and not of the service unit. One video runs on several displays over its life and one display shows several videos over a season, so whoever changes the mode edits the copy the player does not read as soon as the pairing changes.

A 4K capture device advertises 3840×2160 at 30 Hz as its own preferred timing, and the vc4 driver builds no 3840×2160 mode from that advertisement; with the mode forced in `cmdline.txt`, the driver builds that same 297 MHz timing and it works. A 2560×1440 monitor whose EDID never mentions 2160 must carry no force, or the Raspberry Pi transmits a signal the display cannot show. One `video=` token in `cmdline.txt` is therefore right for one venue and wrong for the next, and deleting it changes the target (measured on a Raspberry Pi 4; conditions in [measurements.md](measurements.md)).

The `asset` key completes the pairing of display and video: one file names the venue's display and the video that plays on it. `ExecStart=/usr/bin/dexd` passes no arguments, so the unit states how to run the player and the exhibit config what it plays and where. Several videos can sit in `/opt/dex`, with the config choosing one.

## Config location

dexd looks for `/opt/dex/exhibit.yaml`, then `/opt/dex/exhibit.json`; `--exhibit-config PATH` overrides both. `/opt/dex` is the assets directory, holding the video, its sidecar and the config together; the unit waits for it as a mount point (see [service-unit.md](service-unit.md)).

The package installs no exhibit config: no default file, no file dpkg would preserve across upgrades, no `/etc/dex` directory. A player without one refuses to start and prints the path to create together with a two-line example (see [startup-checks.md](startup-checks.md)).

Exactly one of the two names may exist, and dexd refuses when both do, printing both paths. YAML is searched first, so a `.json` beside it reads as the leftover copy and the message adds `sudo rm <path>` for it, since a config switched from JSON to YAML beside its predecessor is the likely cause. Nothing in the order decides between two files that both exist.

Discovery is the one part of `exhibit.rs` that touches the filesystem, and both dexd and dex-exhibit-apply(1) call it. Two copies would drift, and the apply tool would then reconcile the command line against a file the player does not read.

Discovery separates "not there" from "cannot tell": only a metadata call reporting the file absent means absent, and every other error is reported with its own message.

## File format

The file extension decides the parser: `.json` is read as strict JSON, `.yaml` and `.yml` as YAML, matched without regard to case. dexd refuses an unrecognised or absent extension before reading a byte, whatever the contents would parse as. The extension comes from `Path::extension`, so a dotfile named `.json` has none.

YAML is a superset of JSON, so one YAML parser would read both names; dexd refuses YAML syntax in a `.json` file all the same. Strict JSON inside a `.yaml` file parses identically under both parsers, which lets a tool that writes the config emit one format under either name.

Below the dispatch the two paths share every decision: both produce the flat key/value list the sidecar's JSON parser produces, and one function maps and validates it, so every key name, default, grammar check and message exists once. About forty lines of tree-building are all that differ, and two files expressing the same config parse to equal structures and produce identical messages for the same mistake.

The flat subset carries strings and non-negative integers, one level deep. dexd refuses each of the following, with its own message:

- a nested mapping or a list;
- a `---`-separated multi-document stream;
- a decimal, a boolean or a negative integer;
- a file that is empty or contains only comments, as having no document.

A YAML feature belongs in the subset only if the same config could be written in the `.json` form; one that could not would make a config's meaning depend on the name it was saved under.

dexd refuses anchors and aliases at the parser-event level, before a tree is built, because the loader replaces `*name` with a copy of the anchored node: a loaded document that used an alias is indistinguishable from one that spelled the value out. An anchor with no alias is refused too, since an alias needs an anchor to refer to.

A syntax error is left to the loader, so the operator gets its marked diagnostic and not a vaguer message from the anchor check that runs first.

dexd refuses duplicate keys in both formats: yaml-rust2 errors on inserting one instead of taking the last, and the JSON path uses a hand-written parser callback for the same rule.

Measured against yaml-rust2 0.11: its scalar resolution follows the YAML 1.2 core schema closely, and tests lock the two places where it matters, because a version that adopted YAML 1.1 resolution would change what a deployed config means.

- Only `true` and `false` are booleans, so `kms_force: no` arrives as the string `no`, and dexd refuses it, naming the valid values.
- Null resolution covers `null`, `~` and an empty value, case-sensitively, so `Null` and `NULL` arrive as ordinary strings.

dexd refuses unknown keys, unlike the sidecar's tolerant schema: a refusal names `kms_forse` rather than dropping the forced display mode it was meant to set.

dexd refuses an unparseable config and a missing one alike, and guesses no display.

## Config keys

| Key | Type | Default | Checked against |
|---|---|---|---|
| `asset` | string | none; see Asset resolution | the asset grammar |
| `display_mode` | string | required | the display-mode grammar |
| `kms_force` | string | `none` | the `kms_force` grammar |
| `connector` | string | `HDMI-A-1` | the connector grammar |
| `display` | string | none | type only |
| `venue` | string | none | type only |
| `note` | string | none | type only |

The last three are informational: dexd parses and type-checks them and uses them for nothing else. A wrong type draws a refusal per key, in the same words in both formats: `venue: 2026` and `"venue": 2026` both report that `venue` must be a string, and quoting is the fix in either. What each key means for a technician is in [../guides/reference.md](../guides/reference.md).

## Display mode

`display_mode` is `auto`, or `WxH@R` with W, H and R positive integers — a run of digits containing at least one nonzero, so leading zeros are accepted. The `@R` half is mandatory, because `3840x2160` on its own lets mpv choose among same-resolution timings by list order.

The refresh is an integer, and dexd refuses both fractional forms when it parses the config. Measured on a Raspberry Pi 4 running mpv 0.40 (see [measurements.md](measurements.md)): a rational refresh such as `3840x2160@30000/1001` fails mpv's option parser (`error setting option (-7)`) and produces a restart loop instead of a picture. A decimal such as `3840x2160@29.97` parses and plays the rounded 30 Hz mode that `@30` names, because mpv matches modes by rounded integer refresh. An integer refresh the connector does not offer fails later, at video-output init (see [startup-checks.md](startup-checks.md)).

A connector whose only timing is fractional therefore has no working integer `display_mode` at all. Observed on a Dell U2719DC, whose sole 2560×1440 timing is 59.95 Hz: `2560x1440@60` matches no mode and restarts the player, and `@59.95` rounds to 60 and fails identically. `auto` is the only value that resolves such a connector, because mpv then takes the connector's preferred mode instead of matching a number against the list (see [measurements.md](measurements.md#mode-behaviour-by-sink)).

`kms_force` is `none`, or `WxH@R` with an optional trailing `D`, R again an integer because the kernel's `video=` grammar has no fractional refresh. The `D` is a suffix on the whole token, not part of the refresh.

`D` makes the connector read as connected before a display is attached, which puts the boot-order fix at the KMS layer: a player switched on before its display no longer depends on which came up first. dex-wait-hdmi(1) is the fallback for a connector carrying no force, and storing a copy of the display's EDID for the kernel to read is an optional install-time step documented there.

`connector` is `HDMI-A-<n>` with n a run of digits. dexd refuses lowercase and a trailing space, and accepts a leading zero and `0` itself, because connector numbering is an index, not a rate. The same value spells the `video=<connector>:…` token dex-exhibit-apply(1) writes and the `/sys/class/drm/card*-<connector>` path the mode pre-flight reads (below), so a value accepted loosely here would fail far from where it was typed.

dexd also accepts `--mode WxH@R` on the command line, as a cross-check against `display_mode`. The decision table matches the frame rate's in [sidecar.md](sidecar.md):

| Config | `--mode` | `--test-rig-no-sidecar` | Result |
|---|---|---|---|
| present | absent | no | the config binds |
| present | equal | no | the config binds, cross-checked |
| present | different | no | refused, naming both |
| absent | any | no | refused: no exhibit config |
| any | valid | yes | the command line binds |
| any | invalid | yes | refused, naming the grammar |
| any | absent | yes | the mode is `auto` |

`--test-rig-no-sidecar` (test rig only) also skips the cmdline check, so on that branch the resolved `kms_force` is `none` and the connector the default, since no config was consulted.

Three options carry the result to mpv. `--container-fps-override` takes the resolved frame rate and `--drm-connector` the resolved connector, both on every run. `--drm-mode` takes the resolved display mode unless that is `auto`, in which case dexd passes no such option and mpv applies its own default of the connector's preferred mode.

The mode pre-flight compares the resolution half of `display_mode` against the connector's own mode list in `/sys/class/drm/card*-<connector>/modes` (see [startup-checks.md](startup-checks.md)).

`auto` names no resolution, so dexd skips that pre-flight and compares the asset's width and height, stored in the sidecar, with the connector's mode list instead, warning and naming a forced mode as the repair. Whether that should be a refusal is open; see [Open questions](#open-questions).

## Asset resolution

`asset` is a non-empty path with no trailing slash and no control characters. The asset grammar checks neither existence nor extension: the startup check opens the file and reports the read error, which says more than a grammar refusal would. dexd refuses a control character because it prints this path to the system log at every start, where a newline would forge a second log line.

A relative `asset` resolves against the directory the config file sits in, so `asset: artwork.265` beside `/opt/dex/exhibit.yaml` names `/opt/dex/artwork.265`, and the service's working directory never enters into it. An absolute path is used as given.

| Config `asset` | Command-line path | `--test-rig-no-sidecar` | Result |
|---|---|---|---|
| present | absent | no | the config binds |
| present | equal | no | the config binds, cross-checked |
| present | different | no | refused, naming both |
| absent | present | no | the command line binds, logged as such |
| absent | absent | no | refused: nothing names a video |
| any | present | yes | the command line binds; the config is ignored |
| any | absent | yes | refused: on this branch the path must be on the command line |

That resolution happens before the cross-check, so a relative key and an absolute command-line path naming one file agree.

Nothing defaults to a fixed asset path: a mistyped key would otherwise play the previous season's video, with nothing in the log to show for it. The refusal prints the line to add in both formats, and the package's postinst script names the three files a player needs at install time: the video, its sidecar and the config.

A path given on the command line is a one-off: a test rig, or trying another file on a deployed device without editing the config. dexd logs the source with the path at startup, so a hand-started run cannot be read as evidence about the deployed one.

Because the asset comes from the config, an empty command line is the shipped invocation: a bare `dexd` reaches the config and refuses there when the config names no video.

## Kernel cmdline

dexd compares the config's `kms_force` with the running kernel's command line at every start. `kms_force: none` expects no `video=` token for the connector; any other value expects that exact token. The comparison reads `/proc/cmdline`, because an edit to the file on the boot partition takes effect only at the next boot.

dexd ignores other connectors' `video=` tokens and every other kind of token, so another connector's token counts as absent. If an operator edits one side and forgets the other, dexd refuses at the next start instead of playing on whatever mode the kernel booted with.

Every refusal here names both repairs, because the check cannot tell which side is stale; the reasoning is in [startup-checks.md](startup-checks.md).

A `video=` token with no connector prefix, such as `video=1920x1080@60`, is a shape the kernel's grammar also accepts, and it forces every connector. Neither the check nor the rewrite can say which connector it binds, or how the kernel would arbitrate it against a per-connector token, so both refuse when one is present and quote it verbatim.

The rewrite itself is a pure function in `exhibit.rs`, which dex-exhibit-apply(1) calls. It rewrites the `video=<connector>:…` token for one connector to match `kms_force`, leaving every other token, its position and every other connector's token untouched. A `kms_force` of `none` removes the token; a connector that has none yet gets one appended. Replacing in place, not appending at the end, makes the rewrite idempotent: a second run finds the token already correct and changes nothing.

Two rewrites of one command line for connector `HDMI-A-1`, by the value of `kms_force`:

```
input        console=ttyS0 video=HDMI-A-1:3840x2160@30 rootwait quiet
2560x1440@60 console=ttyS0 video=HDMI-A-1:2560x1440@60 rootwait quiet
none         console=ttyS0 rootwait quiet
```

The rewrite refuses four inputs: text containing an embedded newline, a `kms_force` outside its grammar, a connectorless `video=` token, and a result that would leave the file empty. A trailing newline is trimmed, since an editor may have added one.

## dex-exhibit-apply

dex-exhibit-apply(1) writes the boot command line from the exhibit config. dexd runs as an unprivileged user under `ProtectSystem=strict` and never writes boot config (see [service-unit.md](service-unit.md)), so applying a config change is a separate, privileged step an operator runs by hand:

```
sudo dex-exhibit-apply [--exhibit-config <path>] [--cmdline-path <path>]
```

dex-exhibit-apply checks for root before it touches any file, so a run without it fails early instead of dying half-way through on a permission error. `--cmdline-path` defaults to `/boot/firmware/cmdline.txt` and exists for a test rig. An unrecognised flag, a missing value and `-h` print usage and exit 2.

The tool loads the config through the same discovery dexd uses, one-file rule included, and names the file it read before reporting what it did. A missing config is a plain refusal here, since the tool has nothing else to do. A command line that already matches prints `no change` and exits 0 without writing.

dex-exhibit-apply writes a change in four steps:

1. a backup of the current file, which it fsyncs before touching the original;
2. a sibling temporary file, fsynced;
3. a rename over the original;
4. a best-effort fsync of the parent directory.

The rewritten file keeps the original's trailing-newline convention.

dex-exhibit-apply ignores a failure of the last step, because the data and the file are committed by then. Rewriting in place would truncate first, so a power cut between truncate and commit would leave a zero-length `cmdline.txt` and a Raspberry Pi that will not boot.

Backups are named `cmdline.txt.bak-<seconds since the epoch>`, with a `.1`, `.2` and so on when that name is taken, so two applies in the same second cannot truncate each other's backup. The five newest are kept, because the boot partition is small.

Pruning orders candidates by modification time, never by name: a name sort would delete the newest backup, since a reused unsuffixed name sorts before its own `.1` and `.2` siblings. The backup just written is excluded, because the boot partition's filesystem records modification times to two seconds and can tie two backups from one second. Pruning is best-effort and never fails an apply that has already succeeded.

A first apply, with no old backup to prune:

```
dex-exhibit-apply: applying /opt/dex/exhibit.yaml (connector HDMI-A-1, kms_force 3840x2160@30D)
dex-exhibit-apply: /boot/firmware/cmdline.txt
  old: console=ttyS0 rootwait quiet
  new: console=ttyS0 rootwait quiet video=HDMI-A-1:3840x2160@30D
  backup: /boot/firmware/cmdline.txt.bak-1786968000
REBOOT REQUIRED -- dexd binds the display from the running kernel's /proc/cmdline
```

A run that prunes prints `  pruned old backup: <path>` after the `backup:` line, once per file removed. The last line appears only when the file changed: an unrebooted change has no effect, and the next start's cmdline check reports the mismatch.

| Code | Meaning |
|---|---|
| 0 | The command line was reconciled, or already matched |
| 1 | A file could not be read or written |
| 2 | Refused: not root, an invalid config, or a rewrite the tool refuses (a connectorless `video=` token, an empty or multi-line result) |

## Upgrades and leftovers

dexd does not read a config left under `/etc/dex` by an older installation; the postinst prints a note naming the move when it finds one.

A systemd drop-in is a `.conf` file under `/etc/systemd/system/dexd.service.d/` that overrides unit settings. One that still passes `--mode` either agrees with the config, which is harmless until it is removed, or disagrees, in which case the display cross-check refuses at the next start and names both values. The postinst prints the warning and does not fail the install, since the package must not delete a file an administrator created:

```
sudo rm /etc/systemd/system/dexd.service.d/*.conf && sudo systemctl daemon-reload
```

## Test scope

The grammars, the resolution tables, the cmdline comparator, the rewrite and the pre-flight parser are pure functions; discovery and the write path are not, and the split against the tests that spawn the binary is in [startup-checks.md](startup-checks.md). The list of default paths is injectable, so the discovery policy is testable against a temporary directory.

The tests that spawn the binary use a config fixture of `display_mode: auto` with a synthetic kernel command line carrying no `video=` token, so the cmdline check and the mode pre-flight both pass on any host, whatever its own boot state.

Tests lock in the YAML library properties the schema depends on, so a version bump cannot change what a deployed config means without failing one.

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| One YAML parser for both names | Comments, anchors and unquoted keys accepted inside a `.json` file | Other programs read the file by its name |
| YAML's JSON schema for the `.json` path | Comments, block style and anchors still accepted | The schemas govern scalar resolution, not syntax, and yaml-rust2 offers no selection |
| Anchors and aliases inside the subset | Nested aliases expand exponentially during loading, before any node-type check runs | The one unbounded allocation on the startup path |
| Unknown keys tolerated, as in the sidecar | `kms_forse` drops the forced display mode it was meant to set, with no error | The file has no independent producer to stay compatible with |
| Two config files resolved by precedence | The player reads one file while someone edits the other | The refusal names both files; deleting one is the repair |
| `Path::exists` for discovery | A permission error on a parent directory reads as "no config" | The message would name a file to create that the operator can already see |
| A fractional refresh in `display_mode` | A rational never plays; a decimal plays its rounded integer | Measured on a Raspberry Pi 4 (see [measurements.md](measurements.md)) |
| dexd writing `cmdline.txt` itself | The player would need write access to the boot partition | dexd runs unprivileged and sandboxed, so applying is a separate tool |
| A fixed default asset path | A mistyped `asset` key falls back to the default | It would play the wrong video with no error |

## Open questions

- Whether `auto` should refuse instead of warning when the connector cannot offer the asset's resolution, and whether `auto` should stay the value a fresh config is written with.
- Whether the pre-flight should cover the refresh half as well, by enumerating modes through mpv or parsing them when the command line is applied. An unoffered integer refresh already fails at video-output init, so the extension would only change whether mpv or dexd prints the error, and would catch no new class of mistake.
