# What dexd is

dexd is a gapless video looper for the Raspberry Pi. It plays one HEVC video on the screen or projector of an artwork at a venue and repeats it for weeks, with nobody watching. A dex card goes into a Raspberry Pi, which drives the display over HDMI.

## The loop

dexd repeats the video with no black frame, no stutter and no flash between repeats. A one-second video looping for six weeks looks like a continuous image.

dexd feeds the video to mpv as a stream that never ends: at the last byte the stream starts again at the first, instead of reporting the end of the file. While the video plays, mpv never seeks and never reopens it.

## Unattended operation

dexd starts at boot as a system service, so a player that is only ever switched off at the mains comes back playing. There is no graceful shutdown, and dexd is built to survive that.

While dexd runs, it checks that the picture is still moving, and repairs playback in three steps:

- it reloads the video in place;
- if the picture still does not move, dexd exits and the service manager restarts it two seconds later;
- if dexd stops reporting in at all, a watchdog timer restarts it.

## Requirements on the video

dexd plays one `.265` file and no other format. The video must:

- begin with a keyframe, with no later picture referring back across the start;
- have a sidecar next to it that gives its frame rate and its checksum;
- carry picture only — dexd plays no sound.

dexd is built for 3840×2160 at 30 fps, and smaller videos play too. Higher frame rates are out of scope for now, and dexd does not refuse one: at 3840×2160 a Raspberry Pi 4 decodes 40 fps at 1.08× the speed it plays and 60 fps at 0.753×, so the ceiling lies between them (measured; see the [measurement record](../design/measurements.md)).

See [Prepare your video](prepare-video.md) for the encode settings and commands.

## Files on the player

Three files make a player, all in `/opt/dex`:

```
/opt/dex/artwork.265        the video
/opt/dex/artwork.265.json   its sidecar: frame rate and checksum
/opt/dex/exhibit.yaml       the exhibit config: which video, which display mode
```

The package installs none of them; if one is missing, dexd refuses to start rather than guess and names the file to create.

`/opt/dex` is the assets directory on the player. Copy the three files there over the network and edit the exhibit config in place — [Build a dex card](build-player-card.md) gives the commands. A card whose assets sit on their own partition, editable with the card in a laptop, is planned and not built; see [Roadmap](../design/roadmap.md). See [Configure the exhibit](configure-exhibit.md).

## Hardware

Use a Raspberry Pi 4 with a display or projector on one of its micro-HDMI ports. Earlier boards have no HEVC decoder and cannot play a 4K HEVC video (derived in the [capability record](../design/pi-capability.md)).

- Memory: dexd keeps the whole video in memory, so the board's RAM bounds how large the video may be. The project measured everything on a 4 GB Raspberry Pi 4; 1 GB boards are not tested, and no maximum file size has been measured.
- Raspberry Pi 5: not measured by the project.
- 4K at 60 fps: a Raspberry Pi 4 decodes it more slowly than the video plays (measured; see the [measurement record](../design/measurements.md)). Do not plan it.
- Heat: in a sealed case with no vents the chip peaked 1.6 °C under the temperature at which the firmware slows it down. Measured over two hours on a 2560×1440 video, not on 4K (see the [measurement record](../design/measurements.md)). Give the enclosure vents or a heatsink.
- Display: any HDMI display that can show the mode you configure. Some displays announce 4K and the Raspberry Pi still offers no 4K mode; set the forced display mode in the [exhibit config](configure-exhibit.md).
- Projectors: a projector can wake slower than the Raspberry Pi boots. The service waits for it before starting dexd (`dex-wait-hdmi`).

## Operating system and package

Flash Raspberry Pi OS Lite, 64-bit, the Debian 13 release — Lite, not Desktop. See [Build a dex card](build-player-card.md).

dexd ships as one `.deb` package installed with `apt`, which pulls in the mpv library it needs (`libmpv2` 0.40 or newer) and installs the service. The package, the command and the service all carry the name `dexd`. Install the package; do not build it on the player.

**Note:** dex players built before dexd run a different image that plays up to 1080p. You cannot upgrade that image in place; a dexd player is a new dex card.

## Limits

| Not in dexd | Where it stands |
|---|---|
| Sound | Out of scope; revisit if an exhibition needs it |
| More than one video, playlists, transitions | Planned |
| Copying a new video in from a USB stick | Planned |
| Seeking, scrubbing, remote control of playback | Ruled out by the loop design |

See the [roadmap](../design/roadmap.md) for what is decided but not built.

## Licence

The source code is MIT-0; the project's content — test cards, video masters, branding — is CC0-1.0. Both allow any use, with no attribution and no conditions.

The installed package links the mpv library. Debian builds that library against code under the General Public License, version 3 or later (GPL-3+), so the binary on the player is covered by GPL-3+. If you pass a player on, GPL-3+ applies to whoever receives it.

## Next steps

- [Run, check, troubleshoot](run-check-troubleshoot.md) goes from a symptom to a fix.
- [Reference](reference.md) lists the config keys, exit codes and messages.
- The [glossary](../glossary.md) defines every term used here.
- Manual pages on the player, read with `man dexd`: `dexd`(1), `dex-exhibit-apply`(1), `dex-wait-hdmi`(1). `dex-sidecar` runs on a workstation and has its page in the repository, at `packages/dexd/deploy/man/dex-sidecar.1`.
