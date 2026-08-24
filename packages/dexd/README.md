# dexd

Gapless HEVC video looper for Raspberry Pi, based on [mpv](https://mpv.io/). dexd plays one
.265 video file in an endless loop with no visible break at the loop point: no black frame,
no held frame, no stutter. A short video then runs for weeks in a gallery as one unbroken
picture.

On a Raspberry Pi 4 at 3840×2160, dexd played at the display's rate and reported no dropped
frames through 25 hours 30 minutes unattended (measured; see the
[measurement record](../../docs/design/measurements.md)).

## Requirements

- A Raspberry Pi 4, which decodes 4K HEVC in hardware, and a display.
- Raspberry Pi OS Lite, 64-bit (Debian 13).
- Three files in `/opt/dex` — the video, its sidecar and the exhibit config; the package
  ships none of them.

## Install

Prepare the video and its sidecar on a workstation
([Prepare your video](../../docs/guides/prepare-video.md)) and download the package
([Releases](#releases)). Copy all of it to the player, at the hostname from
[Build a dex card](../../docs/guides/build-player-card.md):

```bash
scp artwork.265 artwork.265.json dexd_*_arm64.deb <user>@<host>.local:/tmp/
```

Then, on the player:

```bash
sudo apt install /tmp/dexd_*_arm64.deb
sudo cp /tmp/artwork.265 /tmp/artwork.265.json /opt/dex/
sudoedit /opt/dex/exhibit.yaml
sudo dex-exhibit-apply
sudo systemctl set-default multi-user.target
sudo systemctl start dexd
```

`set-default multi-user.target` boots the player with no desktop or login session, so
nothing else takes the display. Reboot if `dex-exhibit-apply` printed
`REBOOT REQUIRED`.

A minimal `/opt/dex/exhibit.yaml`:

```yaml
asset: artwork.265
display_mode: auto
```

`auto` takes the display's preferred mode; name a mode such as `3840x2160@30` when you know
it.

The picture appears; `systemctl is-active dexd` prints `active`.

**Note:** set `kms_force` when the display announces 4K but never shows it.
`dex-exhibit-apply` writes that forced display mode into `cmdline.txt` — see
[Configure the exhibit](../../docs/guides/configure-exhibit.md).

## Refusals

dexd refuses to start rather than guess. When the sidecar, the exhibit config or the display
mode is missing or wrong, dexd logs the reason and the fix, then exits with code 2. Read the
refusal with `sudo journalctl -u dexd -n 20`; symptoms and fixes are in
[Run, check, troubleshoot](../../docs/guides/run-check-troubleshoot.md).

## Scope

dexd plays one video file: you cannot skip forward or back, there is no sound, and nothing
appears over the picture.

## Releases

Released packages are attached to their release on the project's
[releases page](https://github.com/KTE/dex/releases). dexd has no release of its own yet
(planned).

Until it has, every change to dexd builds one package,
`dexd_<version>-<build>+g<commit>_arm64.deb`, and CI attaches it to that build's run as the
`dexd-deb` artifact. Under Actions in the [dex repository](https://github.com/KTE/dex),
open the `dexd deb` workflow, pick the newest successful run on `main`, and download and
unzip its `dexd-deb` artifact. Downloading one needs a GitHub account, and GitHub keeps it
only for the repository's retention period, 90 days unless that was changed; where the
newest run's artifact has expired, someone with write access can run the workflow again.

An apt repository for `apt install dexd` is planned — see
[Roadmap](../../docs/design/roadmap.md).

## Documentation

[The documentation index](../../docs/README.md) maps both sets of pages.

1. [What dexd is](../../docs/guides/what-dexd-is.md)
2. [Build a dex card](../../docs/guides/build-player-card.md)
3. [Prepare your video](../../docs/guides/prepare-video.md)
4. [Configure the exhibit](../../docs/guides/configure-exhibit.md)
5. [Run, check, troubleshoot](../../docs/guides/run-check-troubleshoot.md)
6. [Reference](../../docs/guides/reference.md) — config keys, exit codes, refusal messages

[docs/glossary.md](../../docs/glossary.md) defines the technical terms these pages use.

The package installs the manual pages for `dexd`, `dex-exhibit-apply` and `dex-wait-hdmi`;
run `man dexd` on the player. The package does not install `dex-sidecar`: you build it from
source on your workstation ([Prepare your video](../../docs/guides/prepare-video.md)).

Developers start at [Architecture](../../docs/design/architecture.md).

## Licence

The source is MIT-0: use it for anything, with no conditions.

The shipped .deb links Debian's mpv library, which Debian builds against GPL-3+ code.
Whoever redistributes the installed package passes it on under GPL-3+, and must offer the
source of the whole package — see [Packaging](../../docs/design/packaging.md).
