# Configure the exhibit

The exhibit config is the one file that says which video plays and which display mode dexd uses. It sits next to the video, and dexd reads it at every start. The package installs no exhibit config — you write it.

A player needs three files in `/opt/dex`: the video, its sidecar and this config. [Prepare your video](prepare-video.md) makes the first two. Further videos may sit beside them — `asset` names the one that plays.

## The file

dexd looks for `/opt/dex/exhibit.yaml`, then `/opt/dex/exhibit.json`.

Keep one of the two names. If both exist, dexd refuses to start, names both files and tells you to delete the one you do not mean — for example, to keep the YAML file:

```
sudo rm /opt/dex/exhibit.json
```

The file extension decides the format: `.json` is strict JSON (no comments, every name and value in double quotes), `.yaml` and `.yml` are YAML. The keys are the same either way. Write YAML when you want a comment next to a value; write JSON when a program generates the file.

**Note:** dexd refuses YAML in a file named `.json`.

## Example

A minimal config names the video and asks the display for its preferred mode:

```yaml
asset: artwork.265
display_mode: auto
```

A full one, for a display that needs a forced display mode:

```yaml
asset: artwork.265
display_mode: 3840x2160@30
# The graphics driver builds no 4K mode from this display's EDID unless it is forced.
# D: the connector counts as connected before the display is on — for a player switched on at the mains.
kms_force: 3840x2160@30D
connector: HDMI-A-1
display: gallery panel
venue: main hall
note: prepared at 30 fps
```

The same, as JSON (without `display`, `venue` and `note`):

```json
{
  "asset": "artwork.265",
  "display_mode": "3840x2160@30",
  "kms_force": "3840x2160@30D",
  "connector": "HDMI-A-1"
}
```

Write the file with `sudoedit /opt/dex/exhibit.yaml`. Then apply it and start the player:

```
sudo dex-exhibit-apply
sudo reboot   # only if it printed REBOOT REQUIRED
sudo systemctl start dexd   # only if you did not reboot
```

## Keys

| Key | Value | Default |
|---|---|---|
| `asset` | the video to play: a file name next to the config, such as `artwork.265`, or an absolute path | required — the config (or the command line) must name it |
| `display_mode` | `auto`, or `WIDTHxHEIGHT@RATE` with a whole-number rate, such as `3840x2160@30` | required |
| `kms_force` | `none`, or `WIDTHxHEIGHT@RATE` with an optional trailing `D`, such as `3840x2160@30D` | `none` |
| `connector` | the HDMI port the display is plugged into: `HDMI-A-1`, `HDMI-A-2` | `HDMI-A-1` |
| `display`, `venue`, `note` | free text recording what this installation is | none |

Rules that hold for both formats:

- Every value is text, and every key is one of the seven named above. dexd refuses to start on an unrecognised key and names it, so a misspelled `kms_forse` cannot drop a forced display mode.
- `display`, `venue` and `note` change nothing about playback.

Rules for YAML:

- Quote a value that YAML would read as something other than text: `note: "true"`, `venue: "2026"`. dexd refuses a bare `true` or `false` and names the quoting fix; it refuses a bare number, whole or decimal, and names the key.
- Write one set of `key: value` lines per file. dexd refuses a file holding more than one YAML document (two or more sections split by `---`).
- Write each value out. dexd refuses anchors (`&name`) and aliases (`*name`).

## Display mode

The display mode belongs to the installation, not to the video: one player drives a 4K panel, another a 2560×1440 monitor, and the two need opposite settings. Set the display mode for each player when you install that player.

Write the refresh rate as a whole number — `3840x2160@30`, never `@29.97` or `@30000/1001`. dexd refuses both forms: mpv rejects a fraction outright, and a decimal names the whole-number mode it is nearest to, because a display mode carries a whole-number refresh and nothing else (see [Exhibit config](../design/exhibit-config.md#display-mode)).

A whole number must also be a refresh the display offers, and a display may list only a fractional one: one deployed monitor lists its 2560×1440 at 59.95 Hz alone, so `2560x1440@60` matches nothing there and the player restarts over and over, with no forced mode involved. Write `auto` unless the display needs a forced mode: `auto` asks the connector what it offers instead of naming a number that has to match.

`auto` takes the display's preferred mode. When the video's width and height, which its sidecar also records, are not among the modes the connected display offers, dexd writes a warning to the system log and starts anyway. One of two things follows, and the warning names both: the video plays at the wrong size with every reading looking normal, or it never appears and the player restarts over and over. A video larger than every mode the display offers is the second case — there is no setting that rescues it, so prepare the video at a size the display can show.

**Important:** dexd does not check whether the display can show a forced display mode. The Raspberry Pi transmits the forced mode, and the black screen that follows looks like a fault in the player.

Two worked examples:

- A 2560×1440 monitor whose EDID never mentions 2160 shows its own preferred mode correctly: set `display_mode` to `auto` and leave `kms_force: none`. Forcing 4K here transmits a picture the monitor cannot show. Prefer `auto` here to spelling the mode out: one such monitor lists its 2560×1440 at 59.95 Hz alone, and `2560x1440@60` matched no mode and restarted the player (see [Measurement record](../design/measurements.md#mode-behaviour-by-sink)). Give this monitor a video prepared at 2560×1440; the 4K video never reaches the screen on it.
- A 4K capture device announces 3840×2160 at 30 Hz as its preferred mode, and the graphics driver builds no 4K mode from that announcement. Set `display_mode` to `3840x2160@30` and `kms_force` to `3840x2160@30D` — the `D` for a player switched on at the mains, as below. Forced, the same mode works (measured on a Raspberry Pi 4; see [Measurement record](../design/measurements.md#mode-behaviour-by-sink)).

## Forced display mode

`kms_force` fixes the output from boot: the Raspberry Pi drives the mode you name, whatever the display announces. Give it the same mode as `display_mode` for a display that needs a forced mode, and `none` for a display that does not.

A trailing `D`, as in `3840x2160@30D`, makes the connector count as connected before anything is attached. A Raspberry Pi that boots before its display has powered on otherwise reads no EDID, lands on a 1024×768 fallback and stays there. Add the `D` for a player switched on at the mains.

dex-wait-hdmi runs before dexd and waits up to two minutes for a display to report it is connected. For a connector with no forced display mode it is the only safeguard.

## Applying a change

Edit the config and apply it:

```
sudoedit /opt/dex/exhibit.yaml
sudo dex-exhibit-apply
sudo reboot   # only if it printed REBOOT REQUIRED
sudo systemctl restart dexd   # only if it printed no change
```

**Important:** on a player whose root filesystem is read-only — a card set up with an overlay filesystem, which some installations use so that a power cut cannot corrupt them — an edit here is written to the overlay and disappears at the next boot. The player keeps running the config it had. Turn the overlay off, edit, turn it back on; on Raspberry Pi OS that is `sudo raspi-config`, Performance Options, Overlay File System.

dex-exhibit-apply writes the forced display mode into the `video=` option of `cmdline.txt`.

- It leaves every other option in place, including every other connector's `video=` option.
- With `kms_force: none`, it removes that connector's option instead.
- It saves one timestamped backup before it writes.
- It prints `REBOOT REQUIRED` when the file changed, and reports no change when the file already matched.

Run `dex-exhibit-apply` after every edit to `kms_force` or `connector`. An edit that touches only `asset` or `display_mode` takes effect the next time the player starts: `sudo systemctl restart dexd`. See [Run, check, troubleshoot](run-check-troubleshoot.md).

**Important:** do not edit `cmdline.txt` by hand. dexd compares the config with the boot options the Raspberry Pi actually started with, so a hand edit changes nothing until the next reboot, and then dexd finds the two disagree and refuses to start.

## Refusals

dexd refuses to start rather than guess, names what it found and names the fix in the system log (`journalctl -u dexd`); [Reference](reference.md) lists every message. These are the ones that follow an edit.

- No config: the message gives the path to create and the two lines to put in it.
- No video named: the message gives the `asset` line to add, in both formats. dexd does not guess a file.
- A config that disagrees with the boot options: the message names both values and both repairs, since either side may be the stale one. Update the config when the boot options carry a forced display mode this venue needs; run `sudo dex-exhibit-apply` and reboot when the config is the current one.
- A resolution the connected display does not offer: the message lists the resolutions the connector does offer. Either the display is not the one the config was written for, or a forced display mode has not taken effect yet and the player needs a reboot. A refresh rate the display does not offer is not caught here; the player starts and playback fails.

## Command line

The service runs `dexd` with no arguments, so everything comes from the config, and running `dexd` by hand plays whatever the config names.

`--exhibit-config PATH` reads a config from somewhere else. A video path, `--mode` and `--fps` are cross-checks: each must agree with the config (or, for `--fps`, with the sidecar), and if one disagrees, dexd refuses to start and names both values. See `man dexd`.

dexd checks the display settings before it opens the video, so dexd reports a wrong display mode even when the video path is also wrong.

## Older installs

dexd does not read a config under `/etc/dex`. When `/opt/dex` holds no config yet, move it next to the video and remove the empty directory:

```
sudo mv /etc/dex/exhibit.* /opt/dex/
sudo rmdir /etc/dex
```

When `/opt/dex` already holds a config, keep that one and delete the old directory instead — two configs in `/opt/dex` make dexd refuse to start:

```
sudo rm -r /etc/dex
```

Set the display mode in `display_mode`, not in a service override file (a drop-in under `/etc/systemd/system/dexd.service.d/`) that passes `--mode`. Remove the drop-in:

```
sudo rm /etc/systemd/system/dexd.service.d/*.conf && sudo systemctl daemon-reload
```

The package prints each of these as a `NOTE` if it finds one when you install it.

## Related pages

- [Reference](reference.md) — every config key, message and exit code in one place.
- [Prepare your video](prepare-video.md) — making the .265 file and its sidecar.
- [Install dexOS](install-dexos.md) — the recipe that ends where this file is written.
- [Run, check, troubleshoot](run-check-troubleshoot.md) — starting the player and reading the system log.
- [Exhibit config](../design/exhibit-config.md) — how the checks and the reconciliation work.
- Man pages: `man dexd`, `man dex-exhibit-apply`, `man dex-wait-hdmi`.
- [Glossary](../glossary.md).
