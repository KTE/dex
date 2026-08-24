# Run, check, troubleshoot

This page covers starting the player on a prepared card, reading the lines that show it is playing correctly and getting from a symptom to a fix.

dexd reports everything it does to the system log and nothing to the screen. At the venue a refusal, a crash and a sleeping display all look the same: a black rectangle. dexd cannot signal a fault on screen; that is planned and not built. So every diagnosis starts with the log:

```
sudo journalctl -u dexd -n 20
```

## First start

Installing the package enables the service but leaves it stopped. Enabled means the player starts by itself at every boot. Stopped at install means you choose the moment the player takes over the display, instead of it happening under your terminal session.

Run this once the video, the sidecar and the exhibit config are on the card:

```
systemctl is-enabled dexd
sudo systemctl start dexd
sleep 25
systemctl is-active dexd
sudo journalctl -u dexd -o cat | tail -5
```

Expect `enabled`, then `active`, then the startup lines described below.

Before you leave, switch the power off at the wall and on again, and confirm the player comes back playing on its own. The installation is only ever switched off at the wall, so this reproduces the real start condition.

## A healthy start

A good start prints these lines. dexd writes its own lines first; the `mpv/` line arrives once playback begins.

```
dexd 0.1.0 (4f2a9c1b8e30)
dexd: display 3840x2160@30 (exhibit config /opt/dex/exhibit.yaml), connector HDMI-A-1, kms-force 3840x2160@30D
dexd: asset /opt/dex/artwork.265 (exhibit config /opt/dex/exhibit.yaml)
dexd: 812043264 bytes, fps 30 (sidecar), looping endlessly
dexd: watchdog: armed (window 180s, ping cadence 10s)
dexd: heartbeat loops=0 uptime=0s temp=47.1C frame-drops=n/a vo-delayed=n/a pos=n/a pos-age=n/a watchdog=armed pings-dropped=0
mpv/vo/gpu: Using HW-overlay mode. No GL filtering is performed on the video!
```

Read them for four things.

| Line | What to check |
|---|---|
| Version | A real commit code in the brackets, never `(nogit)`. dexd prints this line before anything else, and prints it even when it goes on to refuse. |
| Display and asset | Both lines cite the same exhibit config path. `command line, not the exhibit config` means someone started the player by hand, and the log is not describing the installed setup. |
| Frame rate | The rate is followed by `(sidecar)`. |
| Overlay and watchdog | `Using HW-overlay mode` names the path that sends decoded pictures straight to the display; without it a Raspberry Pi 4 cannot keep up at 4K. `armed` means the service manager is watching the player; a run started by hand prints `inert` instead. |

## The heartbeat line

dexd writes a heartbeat at start and then every ten minutes, so you can still diagnose a fault from the log weeks later. One healthy line looks like this:

```
dexd: heartbeat loops=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=0 pos=3599.4s pos-age=0s watchdog=armed pings-dropped=0
```

| Field | What it reports |
|---|---|
| `loops=` | The loop count |
| `pos=` | The last playback position seen |
| `pos-age=` | How long ago that position was read |
| `pings-dropped=` | Reports to the service manager that could not be sent; an `inert` watchdog prints no count |

After ten minutes of playback a healthy player shows `frame-drops=0`, `vo-delayed=0` and `watchdog=armed`.

`n/a` means dexd has not received a value yet; `off` means mpv turned down dexd's request for that counter — dexd asks for `frame-drops` and `vo-delayed` on every start, so record an `off` and open an issue in the [dex repository](https://github.com/KTE/dex).

**Note:** the heartbeat at start shows `n/a` for `frame-drops`, `vo-delayed` and `pos` — nothing has been decoded at that point. Numbers appear in the next heartbeat, ten minutes later. A line still showing `n/a` for those fields hours into a run means dexd asked mpv for those counters and never got a value; treat the run as unverified and check the decoder.

## Triage

Read the exit code next. `systemctl status dexd` shows the exit code on its `Main PID:` line, and the same line appears in the log.

| Exit code | Meaning | What to do |
|---|---|---|
| 2 | The player refused to start: the invocation, the asset, the sidecar, the exhibit config or the display is wrong. | Read the message, fix the file, start again. A restart on its own changes nothing. |
| 1 | Playback failed while running. | The service manager restarts the player automatically. |
| 0 | The player stopped without a failure. dexd never reaches the end of the video, so nothing in normal operation produces this. | Record the surrounding lines and report them. |

A player that exits 2 and restarts every two seconds with the same message has a problem in its files or its configuration. Read the message once and fix what it names.

## Symptoms and fixes

| Symptom | Likely cause | Fix |
|---|---|---|
| Picture at the wrong resolution, often 1024x768, after a power cut | The display or projector was still asleep when the Raspberry Pi booted, so the system picked a fallback size and kept it | Set `display_mode` and `kms_force` in the exhibit config, run `sudo dex-exhibit-apply`, reboot — see [configure-exhibit.md](configure-exhibit.md). Where the display lists no whole-number refresh, no forced mode can be written for it at all: use `display_mode: auto`, leave `kms_force: none`, and give the connector a saved copy of the display's EDID so its modes are known before the display wakes — see [build-player-card.md](build-player-card.md) |
| dexd refuses right after you edit the config, naming the config and the boot options file | The forced display mode in the config disagrees with the boot options the machine started with. dexd keeps refusing until you apply the change and reboot | The message names both repairs. Run `sudo dex-exhibit-apply` and reboot if the config is right; edit the config instead if the venue needs the mode already in `cmdline.txt` |
| Refuses with `display_mode ... is not among the modes ... offers` | The display cannot show the mode, or a forced mode has not taken effect yet | Check which display is connected; if you have just applied a forced mode, reboot |
| Restart loop with `Could not find mode matching 3840x2160@60` from mpv | The size exists on this display but that refresh rate does not | Set a refresh rate the connector lists, checked against the `modetest` output below. When the list shows only a fractional rate such as 59.95, no whole number will match it and `display_mode: auto` is the answer |
| Restart loop with no mode error, after a warning that the video's size is not among the display's modes | The video is larger than anything this display can show, and the picture never reaches it | Play a video prepared at a size the display lists. No display setting fixes this one — see [prepare-video.md](prepare-video.md) |
| Plays visibly too slow or too fast, with no errors and every counter clean | The sidecar states the wrong frame rate | On the workstation where the video was prepared, write the sidecar again with the correct rate — `dex-sidecar write artwork.265 --fps 30 --force` — then copy the video and its `.json` to the player together; see [prepare-video.md](prepare-video.md) |
| Picture sideways | The .265 records no rotation, so the picture is shown as its pixels were stored, and they were stored turned | Encode from the master again, which turns the picture as it encodes, then confirm the width and height with `ffprobe` — see [prepare-video.md](prepare-video.md) |
| Stutter or dropped frames at 4K | The chip is too hot and throttling, or the decoded pictures are taking the slow path to the display (see below) | Check `frame-drops=`, `vo-delayed=` and `temp=` in the heartbeat, then the hardware-decoding check below; add cooling for a long run |
| Restart loop with `playback ended`, and no message naming a file or a setting | The chip's video decoder became unavailable. dexd refuses to decode on the main processor, so playback fails instead of running slowly | Run the hardware-decoding check below |
| Black screen while the service is active and heartbeats keep coming | The display lost the signal, was switched off, or is on another input | Check the cable, the display's power and its input. dexd cannot see this: decoding and playback continue normally inside the player |

## Connector modes

Ask the connector what it offers before you decide on a mode:

```
edid-decode /sys/class/drm/card*-HDMI-A-1/edid | grep -i "Display Product Name"
modetest -M vc4 -c | grep -m3 "^  #"
```

The first prints the display's own name, the second the sizes and refresh rates it lists. Neither command is part of a default Raspberry Pi OS install; [build-player-card.md](build-player-card.md) names the two packages. The card number changes between kernel versions, so the `card*` pattern matches whichever one this player has.

dexd checks the picture size against the connector and leaves the refresh rate to mpv. A refresh rate the connector does not list therefore reaches mpv, which reports the error when it brings the picture up, and the player restarts.

## Hardware decoding

Check hardware decoding directly when the heartbeat shows dropped frames. On its own, mpv falls back to the main processor with no error when the chip's video decoder is unavailable. dexd forbids that fallback, so a player that loses the hardware path fails and restarts.

Trust the frame counters, not the log line: a run that reports hardware decoding can still drop most of its frames, because the pictures are copied out of the decoder's own format on the way to the display.

Stop the player first, because it is using the display:

```
sudo systemctl stop dexd
ls -l /dev/video*
v4l2-ctl --list-devices
grep -n rpivid /boot/firmware/config.txt
mpv --hwdec=auto --gpu-context=drm --gpu-hwdec-interop=drmprime-overlay \
    --msg-level=all=v /opt/dex/artwork.265 2>&1 | grep -iE 'hwdec|drmprime|Using hardware|fallback|Falling back'
```

`v4l2-ctl` comes from the `v4l-utils` package, which a default Raspberry Pi OS install does not have: `sudo apt-get install -y --no-install-recommends v4l-utils`.

Expect a decode device in the listing, and a line naming hardware decoding through the path that sends decoded pictures straight to the display. Any line about falling back means this hand-run mpv decoded on the main processor.

On Raspberry Pi OS the decoder may need `dtoverlay=rpivid-v4l2` in the boot configuration; the default has changed between releases. The decode device in the listing and the mpv line are the proof. Full per-model detail is in [pi-capability.md](../design/pi-capability.md).

## The HDMI wait

`dex-wait-hdmi` runs before the player at every start and waits for a connector to report that a display is attached, polling once per second for up to 120 seconds. A projector can take 30 to 90 seconds to identify itself while a Raspberry Pi boots in about 15 seconds.

For a projector that needs longer, raise two settings together: `DEX_HDMI_TIMEOUT` for the wait itself, and `TimeoutStartSec=` in the unit, which must stay above it. Run `sudo systemctl edit dexd` and add both:

```
[Service]
Environment=DEX_HDMI_TIMEOUT=240
TimeoutStartSec=270
```

**Important:** if no display reports in, the wait exits with an error and the start fails. The service manager then retries the whole sequence rather than starting the player into a display that is not there. Repeated `no connected HDMI connector after 120s` lines in the log mean the cable, the display's power or its input is the problem.

`dex-wait-hdmi` runs only as the service's pre-start step. Start the player by hand over SSH with no display attached and there is no wait: the player exits within seconds, and the log names the video output that could not start.

## Reference

Every configuration key, log line format, exit code and refusal message with its fix is in [reference.md](reference.md). See also dexd(1), dex-exhibit-apply(1) and dex-wait-hdmi(1).
