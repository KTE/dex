# Roadmap and open questions

This page is for a developer choosing where to contribute: what dexd is planned to grow into, what has been ruled out, and what still needs a decision. Nothing here is implemented unless the text says so; the other pages under `docs/design/` describe current behaviour.

## Origin

dex plays video artworks on Raspberry Pi players in galleries. Its proven gapless loop was hello_video, which needs the legacy Broadcom graphics stack, which ships only on buster and caps output at 1080p.

Output at 3840×2160 is a requirement (decided), and resolution alone disqualifies buster. A buster image can never be security-updated, which suits only a player kept off every network — and the card the guides build joins the venue's network for SSH. buster's inability to run on a Raspberry Pi 5 is a hardware-purchasing question, not a reason for the move.

No published work offered a gapless 4K loop on the current Raspberry Pi graphics stack, so dexd was written. It feeds libmpv an endless byte stream, so the decoder never reaches the end of the file and never seeks (see [endless-stream.md](endless-stream.md)).

Two further requirements are decided. The player must survive having mains power cut, which is how a gallery switches it off. The delivery format is not a constraint: the artist's supplied master (see [artist's supplied master](../glossary.md#artists-supplied-master)) arrives in whatever format the artist authored it in, and the project converts it before it reaches the player (see [../guides/prepare-video.md](../guides/prepare-video.md)).

## Stages

| Stage | State |
|---|---|
| Gapless 4K playback: 3840×2160 at 30 fps, repeating with no visible pause | Built and packaged |
| A pi_video_looper backend for dexd | Decided, not started |
| Ingest on the player: transcode during USB copy | Decided, not started |
| A playlist format: several videos, per-video timing, transforms | Decided, not started |

See [exhibit-config.md](exhibit-config.md) and [packaging.md](packaging.md) for what the first stage consists of; the other three are not specified in detail.

## Player integration

pi_video_looper loads a player backend by module name, so a backend is one Python file.

dexd's backend goes into a fork of the upstream project: a new backend file patches nothing upstream. The fork stays GPLv2.

Once the fork is stable, the project opens an issue asking upstream whether a pull request would be welcome; no dex release waits for the reply.

Which process starts and stops playback is open. The looper's interface assumes a player it starts and stops once per video, while dexd runs until it is killed.

Other options:

- The looper spawns dexd and stops it with a termination signal — the current answer: it matches the other backends and fits one artwork per exhibition. The looper must not restart dexd on exit, which would fight dexd's endless stream.
- dexd stays up and takes new videos over a control channel — needs an interface dexd does not have, and is worth building only once playlists with gapless transitions are required.
- dexd absorbs USB copy, transcoding and playlists — then no looper is left to integrate with.

## Ingest on the player

A technician prepares the video on a workstation and copies it to the dex card over the network. The video, its sidecar and the exhibit config sit together in `/opt/dex`. Giving that directory its own FAT partition, so any computer mounts it and a technician edits the files with the card in a laptop, is the intended arrangement and is not built for a dexd card (see [data partition](../glossary.md#data-partition)); the card the guides build keeps the assets on its root filesystem. dexOS already carries such a partition at `/dexdata`, on an image that plays with pi_video_looper, so what is missing is the arrangement on a dexd card rather than the idea.

The planned stage moves preparation onto the player: during USB copy, the player converts with ffmpeg every file on the stick that has no converted counterpart yet.

That step normalises GOP structure and keyframe placement, which decides whether a file can loop gaplessly. It also records the frame rate, because an elementary stream carries no timestamps (see [sidecar.md](sidecar.md)).

## Playlist format

The playlist stage plays several videos, each with its own timing and transforms such as rotation and mirroring. Both transforms are cheap in mpv (`--video-rotate`, `--vf=hflip`) and cheaper on a KMS plane. The sidecar's flat JSON shape is a plausible starting point. This stage changes which process starts and stops playback, so the answer under [Player integration](#player-integration) should not foreclose it.

## Operating system images

buster stays as the legacy line for existing 1080p players; a trixie line carries 4K with dexd. dexd needs trixie for four things, all of which arrived after buster:

- the Raspberry Pi HEVC decoder driver (see [rpivid](../glossary.md#rpivid))
- the V4L2 stateless request interface
- libmpv 0.40
- trixie's Raspberry Pi-patched ffmpeg

The cost is two bases to maintain, and the legacy line keeps the limitation stated under [Origin](#origin).

The first shipped artwork card is plain Debian trixie with the package. Porting the dexOS patch series to pi-gen's trixie branch is the largest unknown in the image work. Upstream projects stay patched with quilt, so the patch series reads as the list of changes.

## Boot and device work

Boot time is the interval between mains power and the first frame on the wall. With no graceful shutdown, every power cycle is a cold boot in front of an audience (see [service-unit.md](service-unit.md)). The work is planned after 1.0, because it shortens a path that already works.

The method is fixed:

- Measure with `systemd-analyze blame` or `systemd-analyze critical-chain` before changing anything.
- For each cost, ask whether this image needs the thing at all: a card runs one player against one display, so much of what a general-purpose Raspberry Pi OS starts has no consumer. Candidates: the Bluetooth service, swap (`dphys-swapfile`), the wait for a network address.
- Add mount and check options to `cmdline.txt`: `noatime`, `nodiratime`, `data=writeback`, `fsck.repair=yes`.
- Fold the answers into the image build: a service that is never installed cannot cost boot time or come back on an update.

Reordering must not start dexd before something it depends on and rely on restarts to hide the missing dependency. `StartLimitIntervalSec=0` and the restart policy stay as they are.

Boot output stays on screen: a player restarting in front of visitors should show why, so `quiet` stays off the kernel command line. A shutdown button on a general-purpose input pin is planned and not built.

## Failure escalation

dexd recovers in place, and systemd restarts the process when it exits or stops answering the watchdog (see [failure-handling.md](failure-handling.md)).

Reboot escalation is decided and not built: a second systemd unit with a burst counter reached through `OnFailure=` reboots the device after repeated failures inside a window. `StartLimitAction=reboot` is not used, because it interacts badly with `StartLimitIntervalSec=0`, which is what makes dexd retry forever.

## On-site fault signal

The on-site fault signal is decided and not built. Refusals go to the system log only, so at the venue every fault looks the same: a black screen. The signal makes a refusal visible without a laptop.

A privileged `ExecStopPost=` line in the unit will write the last refusal to `/dev/tty1`, because the text console has DRM master back exactly when dexd has refused (see [DRM master](../glossary.md#drm-master)). The escape sequence and which process has the display depend on the kernel and the hardware, so the line is not added until it has been checked on a Raspberry Pi with a projector attached. The troubleshooting guide carries a `journalctl -u dexd -n 20` line in the meantime (see [../guides/run-check-troubleshoot.md](../guides/run-check-troubleshoot.md)).

## Distribution

Setting a player up should be choosing dexOS in Raspberry Pi Imager, the way any other operating system is chosen, so a technician writes one card and has a player. That is the trixie line carrying dexd (see [Two bases](#operating-system-images)); until the image exists, [Install dexOS](../guides/install-dexos.md) reaches the same card by hand. Assembling one by hand stays supported afterwards, for anyone running dexd on a system they chose themselves.

An apt repository on GitHub Pages will serve the package, so a device runs `apt update && apt install dexd` and upgrades work; GitHub Packages carries no Debian format.

The documentation goes to `dex.ars.is/docs`, built with Starlight, a documentation-site generator for Astro, so it deploys with the site already on that domain and needs no subdomain of its own.

Neither exists yet (planned). The project has taken one tag, the 2024 pre-release `v1.0.0-rc.1`, which carries a dexOS image and its build logs; dexd has had no release of its own, and its changelog records `0.1.0-1`.

## Names

The package, the binary and the systemd unit are called `dexd`: `dex` is already a Debian package name. A name like `dex-player` would need a second migration once ingest and playlists arrive. Paths keep the shorter name (`/opt/dex`). A future command to type should be `dexctl`, avoiding a collision at `/usr/bin/dex`.

## Frame rate

dexd targets 30 fps at 3840×2160. Higher rates are out of scope, not ruled out: on a Raspberry Pi 4 at 3840×2160, 40 fps decodes at 1.08× realtime and 60 fps at 0.753×, so the ceiling lies between them (measured; see [measurements.md](measurements.md)). The HDMI capture device the measurements run through offers no 4K mode above 30 Hz, so the question needs a display that accepts a higher rate. Fixing 30 fps now lets the ingest and playlist stages normalise to one rate.

## Out of scope

- Audio. dexd plays silent, and mpv is not silent-only, so audio stays possible — one reason hello_video could not be the long-term player. A projected drift of about 90 s per day between playback position and system uptime (assumed) is invisible in a silent loop but would be an audio-sync defect.
- Frame rates above 30 fps, as under [Frame rate](#frame-rate).
- Hardware other than the Raspberry Pi, such as a small x86 board or a commercial signage player: surviving a power cut is a property of the Pi's design and a firmware setting elsewhere. Revisit only if no Pi-based player works.
- A read-only root filesystem, closed as something the project ships (decided): installations have survived being switched off at the socket without one. That evidence comes from dexOS cards while the card that ships first is plain trixie with the package, so a card that comes back corrupt reopens it. One deployed card now runs with the overlay filesystem switched on by hand and its boot partition write-protected, which is one operator's hardening rather than an arrangement dexd sets up or needs. It is safe there because that card's video never changes; see [Configure the exhibit](../guides/configure-exhibit.md#applying-a-change) for what a read-only root does to an edit.

## Open questions

- Whether `display_mode: auto` should refuse rather than warn when the connector cannot offer the resolution the sidecar states (see [sidecar.md](sidecar.md)), and whether the example in the refusal message should suggest `auto` at all, now that the package installs no config. One deployed player gives evidence on both sides. `auto` was the only working value there, because that panel offers one fractional timing and no whole number matches it; and `auto` is equally what let a 3840×2160 video the panel cannot show reach the player and restart it (see [measurements.md](measurements.md#mode-behaviour-by-sink)).
- A long-running test of the packaged build with real KMS output and the watchdog enabled: the run that showed the watchdog healthy used `vo=null`, so it rendered no picture; no long run covers both (see [measurements.md](measurements.md)).
- The path where a second in-place recovery starts before the first has finished is unexercised on a device: nothing recovered during the twenty-five-hour run (see [measurements.md](measurements.md)).
- Whether the requirement that the file starts with an IDR picture may be relaxed for a particular video; no criteria exist for that judgement.
- A Raspberry Pi 5 leg, budgeted as its own task. The project has no Pi 5 measurements, and whether Raspberry Pi's `+rpt2` ffmpeg build is needed there is unresolved: trixie ships `+rpt1`.
- Whether the trixie line ships one image that selects a display mode or separate 4K and HD images.
- Venting for the enclosure: a sealed passive case is viable but marginal, and a gallery warmer than the room of the sealed-enclosure test consumes the remaining thermal headroom (see [measurements.md](measurements.md)).
- Reading the asset through `mmap` after 1.0, in place of the single in-memory copy dexd makes at startup; the risk is a page fault inside the read callback (see [endless-stream.md](endless-stream.md)) blocking on SD-card input at a frame deadline.
- A development-versus-production toggle read from an input pin at startup, an idea and not built. Configuration needing no write to the filesystem survives a power cut and is visible without a keyboard.
- Rolling the REUSE licensing scheme out to the project's other repositories, and the remaining lint follow-ups: a `cargo clippy -- -D warnings` sweep and a Miri run over the library tests.

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| pivid | Not needed | mpv reached realtime 4K; pivid's 32-bit builds had failed |
| GStreamer with `kmssink` | Blocked upstream | `kmssink` cannot bind a SAND-tiled buffer, an upstream gap and not a misconfiguration (see [architecture.md](architecture.md)) |
| A custom player in Rust, roughly 2000 lines and 10–15 person-days | Held, unneeded | Last of the pre-committed fallback order — pivid, then GStreamer, then this; mpv reached realtime first |
| ffmpeg with `vout_drm` | Kept as a fallback | 1.92× realtime (measured; see [measurements.md](measurements.md)); used only if mpv ever falls short of realtime |
| VLC 4, and WPE WebKit through Cog | Untested since 2024 | Close then; retest before either is considered again |
| Rebasing dexOS onto trixie | Rejected | Would force working 1080p installations to migrate so new ones get 4K |
