# Configure the exhibit

The exhibit config is the one file that says which video plays and which display mode dexd uses. It sits next to the video. You edit it with a text editor, and dexd reads it at every start.

## Where the file lives

dexd looks in the assets directory `/opt/dex` for `exhibit.yaml` or `exhibit.json`. That directory is the part of the dex card any computer can open. Put the card in a computer and edit the config there, next to the video and its sidecar.

The package installs no exhibit config. Without one, dexd refuses to start and writes the reason to the system log (`journalctl -u dexd`), naming the file to create and the two lines it needs at a minimum:

```yaml
asset: loop.265
display_mode: auto
```

Create `/opt/dex/exhibit.yaml` with your video's file name in place of `loop.265` — on the player with `sudoedit /opt/dex/exhibit.yaml`, or on any computer in the top folder of the card's data partition — then run `sudo dex-exhibit-apply` and restart the player with `sudo systemctl restart dexd`.

Keep exactly one of the two files. With both present, dexd refuses to start and names both files, so the player never runs on the file nobody edited. Delete the one you do not mean — for example, to keep the YAML file:

```bash
sudo rm /opt/dex/exhibit.json
```

To read a config from somewhere else, pass `--exhibit-config PATH` — see `man dexd`.

## Two formats, one set of keys

The file extension tells dexd which format to expect, and the keys are the same either way.

- `.json` is strict JSON: no comments, every name and value quoted.
- `.yaml` or `.yml` is YAML: comments allowed, and values need no quotes unless they read as a number or as `true`/`false` (see the YAML rules below).

Choose `.json` when a script writes the file; choose `.yaml` to edit by hand, even from a phone at the venue.

dexd refuses YAML syntax inside a `.json` file, so any other program reading the file as JSON sees what dexd sees.

## Write the file

A 4K display with a forced display mode, in YAML:

```yaml
# This display gets no 4K mode unless it is forced.
asset: artwork.265
display_mode: 3840x2160@30
kms_force: 3840x2160@30
connector: HDMI-A-1
display: 4K capture device
venue: east wall
note: spare card in the technician's drawer
```

The same settings in JSON:

```json
{
  "asset": "artwork.265",
  "display_mode": "3840x2160@30",
  "kms_force": "3840x2160@30",
  "connector": "HDMI-A-1",
  "display": "4K capture device",
  "venue": "east wall",
  "note": "spare card in the technician's drawer"
}
```

## The keys

| Key | Required | Value |
|---|---|---|
| `asset` | yes | The video to play: a file name or relative path, taken from the folder this file is in (`artwork.265`), or an absolute path (`/opt/dex/artwork.265`). No trailing slash, no line breaks or other invisible characters. |
| `display_mode` | yes | `auto`, or a picture size and refresh rate in whole numbers, for example `3840x2160@30`. |
| `kms_force` | no, default `none` | `none`, or the forced display mode: the same size-and-rate form (`3840x2160@30`); add a trailing `D` (`3840x2160@30D`) to treat the display as connected from boot — see Choose the display mode. |
| `connector` | no, default `HDMI-A-1` | The HDMI port the display is plugged into: `HDMI-A-` and a number, for example `HDMI-A-2`. |
| `display`, `venue`, `note` | no | Free text for whoever reads the file next. dexd checks that they are text and does nothing else with them. |

Every value is text. dexd refuses an unknown key and names it, and catches a misspelling such as `kms_forse` at the next start.

Because the config names the video, several videos can sit in `/opt/dex`, with the `asset` line naming the one to play. If neither the config nor the command line names a video, dexd refuses to start and prints the exact line to add, in both formats.

Four rules apply to the YAML form:

- Quote a value that reads as a number or as `true`/`false`. dexd refuses `display_mode: true` and names the fix.
- Write `kms_force: none` when nothing is forced; do not write `no`.
- One document per file: no `---` separators.
- No `&name` / `*name` shortcuts that reuse a value elsewhere in the file; type the value out twice instead.

## Choose the display mode

The display mode belongs to the installation, not to the video. Each display needs its own setting; write it per player when you install the player.

If you force a mode the display cannot show, dexd does not report it: the Raspberry Pi transmits the signal anyway, and the black or garbled screen looks like a fault in the player.

Two displays, opposite settings:

- A 4K capture device announces 3840×2160 at 30 frames per second as its preferred mode, and the graphics driver still offers no 4K mode from that announcement. Set both `display_mode` and `kms_force` to `3840x2160@30`; forced, the same mode works (measured on a Raspberry Pi 4, see the [measurement record](../design/measurements.md)).
- A 2560×1440 monitor describes itself correctly and never lists a 4K mode. It must not carry a forced display mode. Use `display_mode: auto` or `2560x1440@60`, and leave `kms_force` at `none`.

Write the refresh rate as a whole number: `@60`, never `@59.95`. A fraction such as `@30000/1001` stops mpv before it plays anything; mpv rounds a decimal such as `@59.95` to `@60`. dexd therefore refuses both forms when it reads the config (measured on a Raspberry Pi 4).

A display that reports itself late, such as a projector still warming up, can leave the Raspberry Pi on a fallback picture size it never corrects. The trailing `D` on a forced mode makes the Raspberry Pi treat the display as connected from boot, before it is switched on. For a display that may be off or slow when the player boots, write the forced mode with the `D`: `kms_force: 3840x2160@30D`.

The helper that runs before dexd, dex-wait-hdmi, delays each start until an HDMI port reports a display (up to two minutes); with the `D` the port reports one from boot.

`display_mode: auto` uses the mode the display reports as preferred, so there is no mode for dexd to check. When the sidecar records a picture size the connector does not offer, dexd writes a warning to the system log and plays on. Set an explicit mode for a show.

## Apply a change

dexd reads the config at every start and never re-reads it while running, so every change needs a restart.

After any change, run `sudo dex-exhibit-apply`. Reboot when it prints `REBOOT REQUIRED`; otherwise restart the player with `sudo systemctl restart dexd`.

```bash
sudoedit /opt/dex/exhibit.yaml
sudo dex-exhibit-apply
sudo reboot     # only if it printed REBOOT REQUIRED; otherwise sudo systemctl restart dexd
```

`dex-exhibit-apply` reads the same file dexd does and copies the forced display mode into the boot options file, `/boot/firmware/cmdline.txt`, as a `video=` entry, or removes that entry when `kms_force` is `none`. It validates the whole config; `kms_force` and `connector` are the only two keys it acts on.

- It rewrites the `video=` entry for your connector and nothing else: every other option, their order, and any other connector's entry stay as they were.
- It saves one timestamped backup before it writes.
- It prints the old and the new line.
- It prints `REBOOT REQUIRED` only when the file changed.
- When the file already matches the config, it prints that and writes nothing.

Do not edit cmdline.txt by hand; let dex-exhibit-apply change the `video=` entry.

dexd reads the boot options the running system started with, so an applied change takes effect at the next boot.

If the exhibit config and those options disagree, dexd refuses to start and names both values and both repairs, because the stale side can be either one. On a player whose boot options carry a forced mode the venue needs, correct the config; running `dex-exhibit-apply` there would remove the forced mode.

## The command line

The dexd system service starts dexd with no arguments, so everything comes from the exhibit config, and running `dexd` yourself plays whatever the config names.

A video path, `--mode` and `--fps` on the command line are cross-checks rather than overrides. Each must agree with the config or the sidecar; a value that contradicts one makes dexd refuse to start and name both values. Change a setting in the exhibit config, not on the command line.

dexd runs the display checks — missing config, disagreeing boot options, a picture size the connector does not list — before it opens the video.

## After an upgrade

Earlier versions read the config from `/etc/dex`, which dexd no longer opens. Installing the package prints a note when a file is still there; move it next to the video and restart the player:

```bash
sudo mv /etc/dex/exhibit.* /opt/dex/ && sudo rmdir /etc/dex
sudo systemctl restart dexd
```

Then check that the moved file names a video. On a config written before the `asset` key existed, dexd refuses to start and prints the line to add.

A service override file in `/etc/systemd/system/dexd.service.d/` that still passes `--mode` dates from before the display mode moved into the exhibit config. Installing the package prints a note when one is present; remove it:

```bash
sudo rm /etc/systemd/system/dexd.service.d/*.conf && sudo systemctl daemon-reload
```

If the override's mode disagrees with the exhibit config, dexd refuses to start and names both.

## More

- Every key, exit code and refusal message with its fix: [Reference](reference.md).
- Preparing the video and its sidecar: [Prepare your video](prepare-video.md).
- Starting the player and reading its log lines: [Run, check, troubleshoot](run-check-troubleshoot.md).
- Why the display mode belongs to the installation, and how the file is parsed: [Exhibit config](../design/exhibit-config.md).
- Terms used here: [Glossary](../glossary.md).
