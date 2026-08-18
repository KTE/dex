# Configure the exhibit

One file on the player says which video plays and how the display is driven: the exhibit config, `/etc/dex/exhibit.json` or `/etc/dex/exhibit.yaml`. It holds what the installation owns: the picture size and refresh rate to ask the display for, whether the Raspberry Pi should force that mode from boot, and which HDMI port the display is plugged into. dexd reads it at every start, and if the file is missing or invalid it refuses to start rather than guess. A refusal is written to the system log (`journalctl -u dexd`); the screen stays black. See [Run, check, troubleshoot](run-check-troubleshoot.md).

Preparing the video itself is a separate job on a workstation; see [Prepare your video](prepare-video.md). Getting the dex card (the SD card) as far as a booted Raspberry Pi is covered in [Build a dex card](build-player-card.md).

## The file

The package installs `/etc/dex/exhibit.json`, and the package manager keeps your edits to it across an upgrade. As shipped it sets `asset` to `/opt/dex/loop.265`, `display_mode` to `auto` and `kms_force` to `none` — settings that force nothing and are meant to be replaced: set `display_mode` explicitly for a show, and `kms_force` if the display needs it.

Two formats are accepted, and the file extension decides how dexd reads it: `.json` is strict JSON, `.yaml` and `.yml` are YAML. The keys are the same either way. JSON is what the package ships and what a script should write; YAML is easier to edit by hand at the venue, because it takes comments and needs no quotation marks around values such as `3840x2160@30`. Strict JSON inside a file named `.yaml` also parses, since JSON is valid YAML; YAML syntax inside a file named `.json` is refused, so that other tools can still trust the name.

Only one exhibit config may exist. dexd checks for `/etc/dex/exhibit.yaml` and then `/etc/dex/exhibit.json`, but the order picks nothing: if both are there it refuses to start, names both files, and names the fix. A rule that let one win silently is how someone edits a config all afternoon while the player reads the other one.

Switching to YAML is therefore two commands, because the package installed the JSON file, plus a restart, because dexd reads the config only when it starts. `sudoedit FILE` opens the file with permission to save it:

```bash
sudoedit /etc/dex/exhibit.yaml     # write it with the keys below
sudo rm /etc/dex/exhibit.json
sudo systemctl restart dexd
```

Miss the second command and the next start refuses and prints it. To use a file somewhere else entirely, pass `--exhibit-config PATH` — see `man dexd`.

## The keys

| Key | Value | Default |
|---|---|---|
| `asset` | absolute path to the video file, for example `/opt/dex/artwork.265` | none — something must name one |
| `display_mode` | `auto`, or a picture size and whole-number refresh rate, `WIDTHxHEIGHT@RATE`, for example `3840x2160@30` | required, no default |
| `kms_force` | `none`, or the same size-and-rate form, optionally with a trailing `D` | `none` |
| `connector` | `HDMI-A-` followed by a number, for example `HDMI-A-2` | `HDMI-A-1` |
| `display`, `venue`, `note` | free text describing the installation | none |

`asset` names which video plays, as an absolute path with no trailing slash. Because the config names it, several videos can sit in `/opt/dex` and the exhibit config picks one; the file does not have to be called `loop.265`. A relative path is refused: dexd runs as a system service whose working directory is `/`, so a relative path would resolve somewhere nobody typed. If neither the config nor the command line names an asset, dexd refuses to start and prints the line to add in both formats. It does not fall back to `/opt/dex/loop.265`, the path the shipped file names, because that would quietly play last season's video for someone who mistyped the key.

`display_mode` is the mode dexd asks the display for. `auto` takes whatever the display announces as its preferred mode.

`kms_force` is the forced display mode: the mode the Raspberry Pi outputs from boot, instead of trusting what the display announces. A trailing `D`, as in `3840x2160@30D`, additionally makes the port read as connected before anything is attached to it. `dex-exhibit-apply` writes this value into `cmdline.txt`, the Raspberry Pi's one-line file of start-up options; nothing else does.

`display`, `venue` and `note` change nothing. They are where the reason for a setting belongs — the description of the display, where it hangs, why this one needs a forced mode — so the reason is kept in the same file dexd enforces.

A JSON exhibit config for a display that needs a forced 4K mode:

```json
{
  "asset": "/opt/dex/artwork.265",
  "display_mode": "3840x2160@30",
  "kms_force": "3840x2160@30D",
  "connector": "HDMI-A-1",
  "display": "4K HDMI capture device, east wall",
  "venue": "gallery east wall",
  "note": "builds no 4K mode unless the mode is forced"
}
```

A YAML config for a different display, a 2560x1440 monitor that shows its own preferred mode correctly and must not be forced:

```yaml
asset: /opt/dex/artwork.265
display_mode: auto         # this monitor announces its own mode correctly
kms_force: none            # forcing 4K here would show nothing
connector: HDMI-A-1
venue: gallery east wall
note: announces no 4K mode at all
```

## Which display mode a display needs

The display mode belongs to the installation, not to the video file, so it is written per device at install time, ahead of whatever the display and the Raspberry Pi would negotiate at power-on. Two displays showing the same video can need opposite settings.

Getting the mode wrong is silent. The Raspberry Pi transmits a signal the display cannot show, and the result — a black screen, or garbage — looks like a broken player.

Some displays need a forced mode. One HDMI capture device, which looks like a display to the Raspberry Pi, announces 3840x2160 at 30 Hz as its preferred mode, and the graphics driver still builds no 4K mode from that announcement; forced, the identical timing works — a 297 MHz pixel clock, the rate at which picture data is sent (measured; conditions in [the measurement record](../design/measurements.md)). A correct announcement from the display is not enough on its own.

Other displays must not be forced. One 2560x1440 monitor mentions 2160 nowhere in what it announces, so forcing 4K on it transmits a signal it cannot display. Its own preferred mode is right unforced: `display_mode: auto`, or `2560x1440@60` written out, with `kms_force: none`.

Write the refresh rate as a whole number. `@60`, never `@59.95`, and never a fraction such as `@30000/1001`: both forms are refused when the config is read, because a decimal would silently mean its rounded whole number, and a fraction cannot be played at all (measured; see [the measurement record](../design/measurements.md)).

A forced mode also settles boot order. A Raspberry Pi that boots before its display has sent its EDID — the description of itself and its modes — reads no EDID, lands on a fallback size (typically 1024x768) and never corrects itself, which is the normal case for a device switched on at the mains. `kms_force` with a trailing `D` makes the port read as connected from the start, so the Raspberry Pi drives the intended mode whatever order things wake up in. For a port with no forced mode, `dex-wait-hdmi` waits for the display instead.

A named `display_mode` is checked against the modes the connector offers before the video is opened; `auto` cannot be, because there is no named mode to check against. When `display_mode` is `auto` and the sidecar records the video's picture size — `dex-sidecar write` stores it when it can — dexd looks for a mode of that size on the port, and if the port offers none it prints a warning and plays on: the video will run at whatever size the port negotiates, with every other indicator normal. Read the warning as an instruction to set `display_mode` and `kms_force` explicitly.

## Applying a change

Changing the display is two steps, then a reboot only if `dex-exhibit-apply` says so:

```bash
sudoedit /etc/dex/exhibit.json   # or /etc/dex/exhibit.yaml
sudo dex-exhibit-apply
sudo reboot                      # only if it printed REBOOT REQUIRED
sudo systemctl restart dexd      # if it did not ask for a reboot
```

`dex-exhibit-apply` reads the same exhibit config dexd reads and rewrites one thing in `/boot/firmware/cmdline.txt`: the `video=` entry for this connector. Every other option in that file, their order, and any other connector's entry are left untouched, and it writes one timestamped backup before it changes anything. Run it twice, and the second run prints that the file already matches and changes nothing. `dex-exhibit-apply` prints `REBOOT REQUIRED` only when the file actually changed, because dexd binds the display from the start-up options the running system booted with, so an unrebooted change has yet to take effect.

An edit that leaves the start-up options alone — a new `asset`, or a `display_mode` change with no change to `kms_force` — gets that already-matches line and no reboot request. Restart the player yourself then, because dexd reads the exhibit config only when it starts. See `man dex-exhibit-apply`.

Do not hand-edit `cmdline.txt`. `dex-exhibit-apply` owns the `video=` entry, and dexd refuses to start when the exhibit config and the running system's start-up options disagree — which is what an edit with no apply and no reboot looks like. The refusal names both values and both repairs, because it cannot know which side is stale: on a player whose start-up options already carry a forced mode the venue needs, the repair is to update the exhibit config, and running `dex-exhibit-apply` would remove the forced mode instead.

## What the command line can and cannot change

The service starts `/usr/bin/dexd` with no arguments at all: the service definition says how to run the player, and the exhibit config says what it plays and where. Running `dexd` by hand with no arguments does the same thing — it plays whatever the config names.

The arguments that do exist never override the config: a video path given on the command line must agree with the config's `asset`, or supply one when the config names none; `--mode` must agree with `display_mode`; and `--fps` must equal the rate in the sidecar. Any disagreement refuses to start and names both values.

The display checks run before the video file is read. A missing or unreadable exhibit config is refused with the path it looked for, before dexd looks at the asset at all; so is a `display_mode` naming a size this connector does not offer. Only the size half is checked that early: a whole-number rate the display does not offer gets through, and turns up at playback as an error in the system log, with the player restarting in a loop. Every refusal exits with code 2 and names its fix; the full list is in [Reference](reference.md), and reading the system log is covered in [Run, check, troubleshoot](run-check-troubleshoot.md).

## Refusals when the file is read

The set of keys is fixed: an unrecognised key stops the start with an error naming the key, rather than being ignored. A misspelled `kms_forse` stops the start, so a typo cannot silently drop the forced mode you meant. Every value is text — a bare number where a string is expected is refused by name, so `venue: 2026` must be written `venue: "2026"`.

YAML adds three rules of its own. Quote anything that could be read as something other than text: a bare `true` or `false` is read as a yes/no value, not as text, and is refused with the quoting fix. Write one configuration per file — a file split with `---` is refused, since there is no single answer to which part is the config. Repeated values written once and referred to elsewhere (`&name` and `*name`) are refused; type the value out twice instead. Bare `no` stays the text `no`, so it does not become a yes/no value — but the spelling that `kms_force` wants is `none`.

## Leftovers from an older setup

A config written before `asset` existed as a key will refuse to start after an upgrade, since your edits to `/etc/dex/exhibit.json` are kept as they were. dexd never guesses which video to play, so the installer warns during the upgrade, while someone is still sitting in front of the screen. The repair is one line plus a restart:

```bash
sudoedit /etc/dex/exhibit.json
sudo systemctl restart dexd
```

The line to add is the one the installer prints: `"asset": "/opt/dex/loop.265",` — put it directly after the opening `{`, because strict JSON refuses a comma before the closing `}`. `/opt/dex/loop.265` is the path the older setup played from; use the real path if the video has moved since.

An older setup may have added an override file for the service under `/etc/systemd/system/dexd.service.d/` that passes `--mode`. If one is still there, the installer says so; remove it:

```bash
sudo rm /etc/systemd/system/dexd.service.d/*.conf && sudo systemctl daemon-reload
```

A leftover override file that agrees with the exhibit config is harmless until removed; one that disagrees refuses to start, naming both values.

## Related pages

- [Reference](reference.md) — every config key, exit code and refusal message with its fix
- [Run, check, troubleshoot](run-check-troubleshoot.md) — starting the player and reading the system log
- [Exhibit config](../design/exhibit-config.md) — how the format and the checks work, and how `dex-exhibit-apply` keeps `cmdline.txt` in step with the config
- [Glossary](../glossary.md) — the terms used here
- `man dexd`, `man dex-exhibit-apply`, `man dex-wait-hdmi`
