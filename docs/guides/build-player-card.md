# Build a dex card

A dex card is an SD card that turns a Raspberry Pi 4 into a player: the Pi boots from it, takes the display and loops one video. This page takes you from a blank card to a running player.

The recipe covers one Raspberry Pi 4 showing one artwork on one display, unattended, with the mains switch as the only control.

## Hardware and files

- A Raspberry Pi 4 with its power supply, an SD card and the display or projector with its HDMI cable. Use the HDMI port nearest the power socket: that is `HDMI-A-1`, the connector dexd drives unless the exhibit config names `HDMI-A-2`.
- A computer running Raspberry Pi Imager, with an SSH key (create one with `ssh-keygen -t ed25519` if you have none).
- The released dexd `.deb`, your `.265` video and its sidecar. See [Prepare your video](prepare-video.md) for the video and the sidecar.

## Flashing the card

In Raspberry Pi Imager, choose Raspberry Pi OS Lite (64-bit), the Debian 13 release. Take Lite, not Desktop: a running desktop session takes the display, and dexd cannot take it from one.

Set all of the following in Imager's settings, then write the card. Together they make it reachable with no keyboard and no monitor:

| Setting | Value |
|---|---|
| Hostname | a name you will recognise for this player |
| User name | the login you will use over SSH |
| SSH | enabled, public key only — dex players accept no password login; paste your public key (`~/.ssh/id_ed25519.pub`) |
| Wi-Fi | the venue's network, password and country, unless the player has a cable |
| Locale and time zone | the venue's |

Connect the display and switch it on before the first boot. The Raspberry Pi reads the display's description of itself (its EDID) at boot; the EDID gives the connector its list of modes.

## First boot

Put the card in the Pi, power it up and leave it for about two minutes: it resizes its filesystem and reboots once. Then check that you can reach it:

```bash
ssh <user>@<host>.local 'hostname; uptime -p'
```

The player answers with the hostname you set. If `<host>.local` does not resolve, use the address the router shows for the player. Everything below runs over that connection.

## Operating system

Update the system and keep it from starting a desktop:

```bash
sudo apt-get update && sudo apt-get -y full-upgrade
sudo systemctl set-default multi-user.target
```

Setting the system up takes about 20 minutes on a Raspberry Pi 4, most of it waiting on apt.

`multi-user.target` means no desktop starts, so dexd is the only program that takes the display.

Install the two diagnostic tools now if you want them on the card:

```bash
sudo apt-get install -y --no-install-recommends edid-decode libdrm-tests
```

`edid-decode` prints what a connected display reports about itself; `libdrm-tests` provides `modetest`, which lists a connector's modes.

## Installing dexd

Install a package the project built, never one you built yourself: that package is the one every check ran against.

Every change to dexd builds a package, and the build run keeps it as a download. Under Actions in the [dex repository](https://github.com/KTE/dex), open the `dexd deb` workflow, pick the newest successful run on `main`, and download and unzip its `dexd-deb` artifact; it holds `dexd_<version>_arm64.deb`. Downloading it needs a GitHub account, and GitHub keeps an artifact only for the repository's retention period, 90 days unless that was changed; where the newest run's artifact has expired, ask someone with write access to run the workflow again. A releases page and an apt repository that serve the package are planned — see [Roadmap](../design/roadmap.md). Copy the file to the player and install it with apt:

```bash
scp dexd_*_arm64.deb <user>@<host>.local:/tmp/
ssh <user>@<host>.local 'sudo apt-get install -y /tmp/dexd_*_arm64.deb'
```

Use apt, which pulls in the mpv library the player needs; `dpkg -i` installs no dependencies.

The package installs:

- `dexd`, `dex-exhibit-apply` and `dex-wait-hdmi` in `/usr/bin`;
- the service, enabled so the player comes back playing after any power cut, and left stopped so you choose the moment it takes the display;
- the `dex` user the service runs as — under it the player reads `/opt/dex`, opens the display device and writes only its own cache;
- the assets directory `/opt/dex`.

The package ships no video and no exhibit config. Both are content: they change per installation, and changing them should not need a new software release.

Check which build you installed:

```bash
dexd 2>&1 | head -1
```

The first line names the build, for example `dexd 0.1.0 (4f1c8a2b9d3e)`. The code in brackets identifies the source version the build was made from. dexd prints this line and then exits, because no config is on the card yet.

`(nogit)` in place of that code means the build records no source version. Replace the package with a released one.

## Video and config

dexd reads three files from the assets directory `/opt/dex`: the video, its sidecar and the exhibit config. That directory belongs to root, so copy through `/tmp`:

```bash
scp artwork.265 artwork.265.json <user>@<host>.local:/tmp/
ssh <user>@<host>.local 'sudo mv /tmp/artwork.265 /tmp/artwork.265.json /opt/dex/'
```

Now write `/opt/dex/exhibit.yaml`, naming that video and the display mode — see [Configure the exhibit](configure-exhibit.md) for the keys and the rules. Then apply it:

```bash
sudo dex-exhibit-apply
```

`dex-exhibit-apply` writes the forced display mode into `cmdline.txt` and prints `REBOOT REQUIRED` when the file changed.

Reboot if it printed `REBOOT REQUIRED`; the service comes up on its own. Otherwise start the player yourself:

```bash
sudo systemctl start dexd
```

The player takes the display and the video starts looping.

dexd refuses to start rather than guess when the sidecar or the exhibit config is missing, and writes the reason to the system log. Read it with `sudo journalctl -u dexd -n 20` — see [Run, check, troubleshoot](run-check-troubleshoot.md).

## Display and boot order

A player and its display switch on together at the mains, and the display is usually the slower of the two. A projector can take 30–90 seconds to present its EDID while the Pi boots in about 15. With no EDID to read, the Pi falls back to a small mode, typically 1024×768, and stays there: it does not change mode when the display finally answers.

Three safeguards prevent that fallback:

- Forced display mode. Set `kms_force` in the exhibit config and run `sudo dex-exhibit-apply`. Give the mode a trailing `D` (`3840x2160@30D`) and the connector reports a display as attached before one is plugged in, so boot order stops mattering. See [Configure the exhibit](configure-exhibit.md).
- `dex-wait-hdmi`, installed with the package and run by the service before the player — no setup. The script waits up to two minutes for a connected display, then fails the start so the service retries the whole sequence. The wait covers a connector with no forced mode, a swapped cable and a replaced projector. See `man dex-wait-hdmi`.
- A saved copy of the display's EDID. Optional. Capture it once while the display is awake and connected:

  ```bash
  sudo mkdir -p /lib/firmware/edid
  cat /sys/class/drm/card*-HDMI-A-1/edid | sudo tee /lib/firmware/edid/dex.bin > /dev/null
  ```

  Name that file in the `drm.edid_firmware` option. The Raspberry Pi looks for firmware files under `/lib/firmware`, so the option names it `edid/dex.bin`.

  [Configure the exhibit](configure-exhibit.md) tells you not to edit `cmdline.txt` by hand. That rule covers the `video=` option: `dex-exhibit-apply` writes it from the config, and dexd refuses to start when the two disagree. `drm.edid_firmware` is the one option you add yourself, once. Add it to the end of the line, separated by a space:

  ```bash
  sudoedit /boot/firmware/cmdline.txt
  # add: drm.edid_firmware=HDMI-A-1:edid/dex.bin
  ```

  Name the connector the display is plugged into: `HDMI-A-2` in both the capture command and the option if the display is on the second port.

  **Important:** `cmdline.txt` is one line. The Raspberry Pi reads nothing after the first line, so an option on a second line does nothing.

  Reboot, then confirm the line took effect with `cat /proc/cmdline`.

## Upgrading dexd

Install the newer `.deb` the same way you installed the first one. Each released build carries its own version, `0.1.0-<build>+g<source-version>`, so apt sees it as newer and upgrades.

**Important:** apt skips a package whose version equals the installed one and prints no warning. To install a package that carries the version already on the card, add `--reinstall`.

`dexd --version 2>&1 | head -1` reports the version on any card, config or not: dexd writes that line before it reads anything, then prints the usage text and exits 2 without taking the display. For the package version rather than the source one, run `dpkg-query -W dexd`. To see a running player's own startup lines instead, read the log:

```bash
sudo systemctl restart dexd && sudo journalctl -u dexd -n 20
```

The first line dexd prints after the restart names the source version the running build was made from. For the package version, run `dpkg-query -W dexd`.

## Related pages

- [What dexd is](what-dexd-is.md) — what the player does and what it needs.
- [Prepare your video](prepare-video.md) — making the `.265` file and its sidecar.
- [Configure the exhibit](configure-exhibit.md) — the config file, its keys and its refusals.
- [Run, check, troubleshoot](run-check-troubleshoot.md) — starting the player and reading the log.
- [Reference](reference.md) — command lines, config keys, exit codes and every refusal message.
