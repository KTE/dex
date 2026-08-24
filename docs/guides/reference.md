# Reference

Every command line, key, log line, file path, exit code and refusal message in one place; the guides listed at the end show how to use them.

dexd writes its lines to the system log; read them with `journalctl -u dexd`. `dex-sidecar` and `dex-exhibit-apply` print theirs to the terminal you run them from.

## dexd command line

```
dexd [<stream.265>] [--fps <F>] [--mode WxH@R] [--exhibit-config PATH] [--test-rig-no-sidecar] [--no-defaults] [--opt K=V ...]
```

The service runs `dexd` with no arguments; the exhibit config supplies the rest.

| Option | Meaning |
|---|---|
| `<stream.265>` | the video to play; optional, since the config's `asset` key names it. Given here too, it must match the path the config resolves to. |
| `--fps <F>` | cross-check on the frame rate: `30`, `29.97` or `30000/1001`; it must equal the sidecar value. |
| `--mode WxH@R` | cross-check on the display mode; it must equal the config's `display_mode`. |
| `--exhibit-config PATH` | read the config from `PATH` instead of `/opt/dex`. |
| `--opt K=V` | pass one more option to mpv, applied after the built-in set, so it replaces a built-in value of the same name. Repeatable. |
| `--no-defaults` | leave out dexd's whole built-in mpv option set — hardware decoding, the display output, fullscreen and the timestamps a `.265` file needs. dexd's own tests use it with `--opt vo=null`; a deployed player never passes it. |

If a flag's value is missing, dexd prints the usage text and exits 2: a service file that lost a value never starts the player at the wrong rate or size.

Four more flags exist for testing: three for a test rig — a Raspberry Pi set up for testing, never a player at a venue — and one for dexd's own test suite.

| Flag | Meaning | Requires |
|---|---|---|
| `--test-rig-no-sidecar` | skip the sidecar and the config; dexd takes `--fps` and `--mode` as given, and `--mode` defaults to `auto` | `--fps` |
| `--test-rig-force-recovery-after-secs N` | force an in-place recovery — dexd reloads the video without restarting the process — `N` seconds after the video is queued | `--test-rig-no-sidecar` |
| `--test-rig-hang-after-secs N` | stop responding `N` seconds after startup, standing in for a hang in dexd's own code | `--test-rig-no-sidecar` |
| `--proc-cmdline PATH` | a stand-in boot-options file instead of `/proc/cmdline`, for dexd's own test suite; the usage text does not list it | — |

**Important:** never pass a `(test rig only)` flag on a deployed player: each one makes the player fail on purpose. Neither timer flag can fire against a video bound to its sidecar.

## dex-sidecar command line

Run this on the workstation where you prepare the video. The package does not install it.

```
dex-sidecar write <stream.265> [--fps <F>] [--out <file>] [--force]
dex-sidecar check <sidecar.json> <stream.265>
```

| Option | Meaning |
|---|---|
| `--fps <F>` | the frame rate to record: `30`, `29.97` or `30000/1001`. Without it, ffprobe reads the rate from the video, which works only if the encoder wrote timing into it. |
| `--out <file>` | where to write the sidecar. Default `<stream.265>.json`, the only name dexd looks for. |
| `--force` | proceed past two refusals: replacing a sidecar that exists, and an `--fps` that disagrees with ffprobe. |

`check` parses the sidecar with dexd's own reader and recomputes the checksum; on success it prints `OK fps=… sha256=… width=… height=…`.

## dex-exhibit-apply command line

```
sudo dex-exhibit-apply [--exhibit-config <path>] [--cmdline-path <path>]
```

dex-exhibit-apply writes the forced display mode from the config into the `video=` option of `cmdline.txt`, leaves every other option untouched, and saves one timestamped backup first. `--cmdline-path` (test rig only) defaults to `/boot/firmware/cmdline.txt`; dex-exhibit-apply prints the usage text and exits 2 for any other argument.

## dex-wait-hdmi command line

`dex-wait-hdmi` takes no arguments and runs before dexd at every start. It polls the connectors once a second until one reports a display, for up to 120 seconds; `DEX_HDMI_TIMEOUT` sets that limit.

## Exit codes

| Program | 0 | 1 | 2 |
|---|---|---|---|
| `dexd` | does not occur in normal operation | mpv could not be created, initialised or given the video; playback failed while running; or in-place recovery ran out of attempts. The service manager restarts the player | dexd refused before playback; the message names which check failed: the command line, the config, the boot options, the display, the video or the sidecar. Restarting cannot help |
| `dex-exhibit-apply` | `cmdline.txt` written, or already correct | a file could not be read or written | refused: not run as root, an unusable config, or a rewrite that would leave `cmdline.txt` empty or on more than one line |
| `dex-sidecar` | done | the video could not be read, or verification failed | bad command line, or a refusal |
| `dex-wait-hdmi` | a display reported in | nothing reported in before the limit; the start fails and is retried |  |

dexd exits 2 when mpv rejects an option, not 1: the same command line fails identically on every restart.

## Exhibit config keys

The file is `/opt/dex/exhibit.yaml` or `/opt/dex/exhibit.json`, and only one of the two may exist. The package installs neither; create the file yourself. Every value is text in both formats; quote a value that reads as a number or a boolean. A key not in this table makes dexd refuse to start.

| Key | Value | Default |
|---|---|---|
| `asset` | a file name or relative path, resolved against the directory the config is in, such as `artwork.265` beside `exhibit.yaml`, or an absolute path | none; the config or the command line must name one |
| `display_mode` | `auto`, or `WIDTHxHEIGHT@RATE` with a whole-number rate, such as `3840x2160@30` | required |
| `kms_force` | `none`, or `WIDTHxHEIGHT@RATE` with an optional trailing `D`, such as `3840x2160@30D` | `none` |
| `connector` | `HDMI-A-<n>` (the Raspberry Pi 4 has `HDMI-A-1` and `HDMI-A-2`) | `HDMI-A-1` |
| `display`, `venue`, `note` | free text; the player checks the type and reads them nowhere else | none |

Both formats carry the same keys:

```yaml
asset: artwork.265
display_mode: 3840x2160@30
kms_force: 3840x2160@30
connector: HDMI-A-1
display: gallery panel
venue: east wall
note: forced mode, see the config
```

```json
{"asset":"artwork.265","display_mode":"3840x2160@30","kms_force":"3840x2160@30","connector":"HDMI-A-1","display":"gallery panel","venue":"east wall","note":"forced mode, see the config"}
```

## Sidecar keys

The sidecar is `<video>.json`, next to the video, written by `dex-sidecar write`.

```json
{"fps":"30","sha256":"<64 hex digits>","width":3840,"height":2160}
```

| Key | Value | Required |
|---|---|---|
| `fps` | the frame rate as text: `"30"`, `"29.97"`, `"30000/1001"` | yes |
| `sha256` | the video's checksum, 64 hex digits | yes |
| `width`, `height` | whole numbers, used for the warning when `display_mode` is `auto` | no |
| `source`, `encoder_cmd` | free text recording where the video came from | no |

Keys not listed here are ignored, so a tool may add its own.

## System log lines

At every start dexd prints these lines, in order:

```
dexd 0.1.0 (<12-hex commit>)
dexd: display 3840x2160@30 (exhibit config /opt/dex/exhibit.yaml), connector HDMI-A-1, kms-force 3840x2160@30D
dexd: asset /opt/dex/artwork.265 (exhibit config /opt/dex/exhibit.yaml)
dexd: <n> bytes, fps 30 (sidecar), looping endlessly
dexd: watchdog: armed (window 180s, ping cadence 10s)
```

The first line names the version and the build it came from; dexd writes it to standard error before anything else, so `dexd --version 2>&1 | head -1` reports the installed version. `--version` is not an option: the command also prints the usage text and exits 2.

In place of the config path, the display line names `test rig override, unbound` when dexd skipped the config. The asset line names `command line, not the exhibit config` when you gave a path on the command line.

The watchdog line reads `dexd: watchdog: inert (<reason>)` when dexd is not answering a timer. Outside the service that is normal; the reason says which case it is.

Lines beginning `mpv/` come from mpv, passed through as they are.

The heartbeat follows at start and every ten minutes:

```
dexd: heartbeat loops=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=2 pos=3599.4s pos-age=0s watchdog=armed pings-dropped=0
```

| Field | Meaning |
|---|---|
| `loops` | the loop count |
| `uptime` | seconds since the process started |
| `temp` | the chip temperature |
| `frame-drops` | mpv's count of dropped frames |
| `vo-delayed` | mpv's count of late frames |
| `pos` | the last playback position |
| `pos-age` | seconds since that position was read |
| `watchdog` | `armed pings-dropped=N` under the service, `inert` elsewhere |

`n/a` is a value that has not arrived yet, `off` a counter the player never asked for. [Run, check, troubleshoot](run-check-troubleshoot.md) reads a healthy line field by field.

## Files and paths

| Path | Holds |
|---|---|
| `/usr/bin/dexd`, `/usr/bin/dex-exhibit-apply`, `/usr/bin/dex-wait-hdmi` | the installed programs |
| `/opt/dex` | the dex card's data partition — the one any computer can open: the video, its sidecar and the config |
| `/opt/dex/exhibit.yaml` or `/opt/dex/exhibit.json` | the exhibit config; the package installs none, so create it |
| `/boot/firmware/cmdline.txt` | the boot options, including the forced display mode |
| `/boot/firmware/cmdline.txt.bak-*` | backups from `dex-exhibit-apply`; the five newest are kept |
| `/var/cache/dexd` | the service's writable directory, created by the service manager, removed when the package is purged |
| `/usr/share/man/man1/`, `/usr/share/doc/dexd/` | the man pages and the package's documentation file |

The service is `dexd.service`: it waits for `/opt/dex` to be mounted, runs as user `dex` in the groups `video` and `render`, and restarts the player two seconds after any exit.

## Messages: exhibit config

dexd prints one of these and exits 2.

| Message begins | Meaning and fix |
|---|---|
| `no exhibit config: dexd refuses to guess the display` | no config in `/opt/dex`; the package installs none, so create it — the message prints the path and the two lines to put in it |
| `2 exhibit configs exist at once` | delete the one you do not mean |
| `--exhibit-config "…": no such file` | correct the path, or drop the flag to use `/opt/dex` |
| `cannot stat …` | a directory above the file cannot be entered by the `dex` user |
| `cannot read …: the file exists but is not readable` | let the `dex` user read the file |
| `cannot tell the format from the file name` | rename the file to end in `.json`, `.yaml` or `.yml` |
| `exhibit config: missing required key "display_mode"` | add `display_mode: auto`, or the mode this display needs |
| `exhibit config: invalid display_mode` | write `auto` or `3840x2160@30`, never `@29.97` |
| `exhibit config: invalid kms_force` | write `none`, or a mode with an optional trailing `D` |
| `exhibit config: invalid connector` | write `HDMI-A-<n>`; the Raspberry Pi 4 has `HDMI-A-1` and `HDMI-A-2` |
| `exhibit config: invalid asset` | a trailing slash or a control character; write a file name or an absolute path |
| `exhibit config: … must be a string` | a number where text belongs; quote it, as in `venue: "2026"` |
| `exhibit config: unknown key` | a misspelled key; the message lists the seven |
| `exhibit config YAML: … resolved to a boolean` | a bare `true` or `false`; quote it |
| `exhibit config YAML: … has a … value` | a list, a nested block or an empty value; give the key one line of text |
| `exhibit config YAML: … has negative value …` | the file format carries text and whole non-negative numbers only; quote the value |
| `exhibit config YAML: key is a …, expected a string` | a key that is not plain text; write `key: value` lines |
| `exhibit config YAML: no document` | the file is empty or holds only comments |
| `exhibit config YAML: 2 documents in one file` | remove the `---` separator and keep one set of keys |
| `exhibit config YAML: top level is a …` | write the file as `key: value` lines |
| `exhibit config YAML: uses an alias` / `declares an anchor` | dexd refuses `*name` and `&name`; write the value out |
| `--mode … contradicts exhibit config display_mode` | drop `--mode`; the config decides |
| `command-line asset … contradicts the exhibit config's asset` | drop the path; the config decides |
| `no asset:` | nothing names a video; add the line the message prints |
| `--mode … is not a valid display mode` | under `--test-rig-no-sidecar`, `--mode` is the only source of the display mode; write `auto` or `3840x2160@30` |
| `--test-rig-no-sidecar consults no exhibit config, so the asset must be given on the command line` | name the video on the command line |
| `--test-rig-no-sidecar requires an explicit --fps` | under `--test-rig-no-sidecar` no sidecar supplies the rate; add `--fps` |
| `--test-rig-force-recovery-after-secs requires --test-rig-no-sidecar` / `--test-rig-hang-after-secs requires --test-rig-no-sidecar` | add `--test-rig-no-sidecar --fps <F>`, or drop the timer flag |
| `--fps "…" is not a valid frame rate` | write `30`, `29.97` or `30000/1001` |

## Messages: display

| Message begins | Meaning and fix |
|---|---|
| `exhibit config says kms_force=…, but the kernel cmdline carries …` | the config and the boot options disagree, and either may be the old one, so the message names both repairs: run `sudo dex-exhibit-apply` and reboot, or edit `kms_force` |
| `the kernel cmdline carries a connectorless token` | a `video=` option in `cmdline.txt` names no connector and drives all of them; write `video=HDMI-A-1:…` or remove it, then reboot |
| `display_mode "…" is not among the modes … offers` | the message prints the sizes the display does offer. Connect the display the config was written for, or reboot if a forced display mode has not taken effect |
| `display_mode "…" requested for connector …, but no … modes list was found` | no such connector on this device; check the `connector` key |
| `cannot read /proc/cmdline` / `cannot read /sys/class/drm/…` | the boot options or the display's mode list could not be read |
| `warning: display_mode is "auto" and the asset is …` | the display cannot offer the video's own size. Either the picture plays at whatever mode the display and the Raspberry Pi settle on, or it never appears and the player restarts. If the display can show that size once forced, set `display_mode` and `kms_force`, run `sudo dex-exhibit-apply`, reboot; if it cannot, prepare the video at a size the display lists |

The warning is the only line here that lets the player start.

## Messages: video and sidecar

| Message begins | Meaning and fix |
|---|---|
| `cannot read …` | the video is not there; correct the `asset` key, or copy the file across |
| `… is empty` | a zero-length file; copy the video again |
| `cannot read sidecar ….json` / `no sidecar found; refusing to guess the frame rate` | run `dex-sidecar write` and copy both files |
| `asset does not match its sidecar: sha256 …` | the two are not a pair: one is old, wrong or half-copied |
| `--fps … contradicts sidecar fps …` | drop `--fps`; the sidecar decides |
| `sidecar: missing required key "fps"` / `"sha256"` | write the sidecar again |
| `sidecar: fps must be a JSON string` | quote the rate, as in `"fps":"30"`, so `30000/1001` survives |
| `sidecar: invalid fps` | write `30`, `29.97` or `30000/1001` |
| `sidecar: sha256 must be 64 hex digits` | a truncated or edited checksum; write it again |
| `sidecar: width/height must be integers` | remove the quotes around the numbers |
| `duplicate key` / `unsupported value for` | a hand-edited sidecar; write it again with `dex-sidecar write` |
| `first slice appears before parameter sets` | the video does not open with its own description; encode it again |
| `leading keyframe is CRA (open GOP), not IDR` | later pictures refer back past the first one, to frames that are not there, so the picture would break at the loop point; encode again with a keyframe at frame 0 |
| `first slice NAL is type …, not an IDR` | the file does not start on a complete picture, often because it was cut; encode from the master again |
| `no Annex-B start code found` | this is not a .265 file; the message prints the ffmpeg command that makes one |

## Messages: playback

dexd exits 1 after every `fatal:` line, and the service manager restarts it.

| Message begins | Meaning and what follows |
|---|---|
| `dexd: health check: … attempting in-place recovery 1/3` | the health check — dexd's ten-second test that playback is still advancing — found it stopped, so dexd reloads the video inside the running process; three attempts, then exit 1 |
| `dexd: health check: in-place recovery's loadfile replaced the stream; absorbing the expected END_FILE(reason=stop) …` | the reload ended the file it replaced; dexd expects that event and playback carries on |
| `dexd: fatal: in-place recovery exhausted its budget` | the three attempts changed nothing |
| `dexd: fatal: playback ended (reason=…, error=…)` | playback reached an end, which an endless loop never does |
| `dexd: fatal: mpv event queue overflowed` | mpv dropped at least one event, which may have been the one that mattered |
| `warning: the health check is disabled for this run` | the health check does not run this time; playback continues, and a fatal event still restarts the player |
| `warning: --test-rig-force-recovery-after-secs elapsed but cannot fire` | the health check is off this run, so `--test-rig-force-recovery-after-secs` has no recovery attempt to spend |
| `warning: (test rig only) (forced-recovery probe): --test-rig-force-recovery-after-secs=… is armed` | this run forces a recovery whether or not anything stalled |
| `warning: (test rig only) (hang probe): --test-rig-hang-after-secs=… is armed` | the player stops responding after the time given |
| `error: dexd: watchdog: … Exiting now so Restart= retries cleanly instead` | the timer is running and the player cannot answer it; exit 1 and a restart |
| `warning: dexd: watchdog: … running without watchdog pings for this run` | no watchdog timer is running, which is normal outside the service |
| `warning: dexd: watchdog: $WATCHDOG_USEC is set …` | the timer expects answers from another process id; expect repeated kills until the service file is fixed |
| `dex-wait-hdmi: no connected HDMI connector after 120s` | check the cable, the display's power and its input |
| `error: mpv_create failed` / `error: mpv_initialize: …` / `error: mpv_stream_cb_add_ro: …` / `error: loadfile: …` | mpv could not be set up or given the video; exit 1 |
| `warning: mpv_command_async(loadfile) …` | mpv refused the reload an in-place recovery asked for, so that attempt changes nothing |
| `error: set <option>=<value>: …` | mpv rejected an option. If you passed it with `--opt`, remove that flag; otherwise the installed mpv does not have the option dexd asked for — check the mpv version. Exit 2 |

## Messages: dex-exhibit-apply

| Message | Meaning |
|---|---|
| `dex-exhibit-apply: applying <path> (connector …, kms_force …)` | the first line of every run: which config it read |
| `error: dex-exhibit-apply must run as root` | run it with `sudo` |
| `error: no exhibit config found (looked for …)` | create the config, or name one with `--exhibit-config` |
| `dex-exhibit-apply: … already matches the exhibit config -- no change` | nothing written, no reboot needed |
| `REBOOT REQUIRED` | `cmdline.txt` changed; the old line, the new line and the backup print above it |
| `error: cannot write backup …` / `cannot rename …` | the boot partition could not be written; `cmdline.txt` is untouched |
| `cmdline.txt carries a connectorless token` | correct that option by hand, then run this again |

## Messages: dex-sidecar

| Message | Meaning |
|---|---|
| `error: not a file: …` / `… is empty` / `error: cannot read <stream>` | the video named is missing, a directory, or unreadable; check the path |
| `error: … already exists; pass --force to replace it` | a sidecar is already there |
| `error: --fps "…" is not a frame rate dexd can read` | write it as `30`, `29.97` or `30000/1001` |
| `error: --fps X disagrees with Y, the rate ffprobe read from …` | pass `--force` to keep yours, or drop `--fps` for ffprobe's |
| `error: no frame rate for …, and none could be read from the stream` | no timing in the video; pass `--fps` |
| `note: frame rate … read from …'s own headers` | ffprobe supplied the rate |
| `error: the sidecar just built was rejected by dexd's own parser` / `reads back as fps … sha256 …` | dex-sidecar parses the text it built with dexd's own reader before writing it, then writes a temporary file and renames it into place; on a mismatch it writes nothing |
| `error: cannot write <out>` | the sidecar could not be written; check the directory it names |
| `wrote <file>` | the sidecar is on disk |
| `error: cannot read <sidecar>` | `check` could not read the sidecar; correct the path |
| `error: <sidecar>: …` | `check` could not parse the sidecar; write it again with `dex-sidecar write` |

## Related pages

- [What dexd is](what-dexd-is.md), [Build a dex card](build-player-card.md), [Prepare your video](prepare-video.md), [Configure the exhibit](configure-exhibit.md), [Run, check, troubleshoot](run-check-troubleshoot.md).
- dexd(1), dex-exhibit-apply(1), dex-wait-hdmi(1).
- [Startup checks](../design/startup-checks.md) and [Failure handling at runtime](../design/failure-handling.md) — the order of the checks and what recovery does.
- [Measurement record](../design/measurements.md) — how the playback numbers in the other guides were measured.
- [Glossary](../glossary.md).
