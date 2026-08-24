# dex

dexd is built to play the video of an installation on a Raspberry Pi, unattended, for weeks. This repository
holds dexd and the parts around it.

If you are setting up a player, start with [What dexd is](docs/guides/what-dexd-is.md). [The documentation index](docs/README.md) maps both sets of pages, for a technician and for a developer.

## Packages

| Path | What it is |
|---|---|
| [`packages/dexd`](packages/dexd/README.md) | The player: gapless 4K HEVC looper for Raspberry Pi 4, shipped as a Debian package. Documentation in `docs/guides/` and `docs/design/`. |
| `packages/dex-os` | dexOS, dex's own Raspberry Pi OS image; it currently plays videos with pi_video_looper, not dexd. |
| `packages/example-content` | The test cards: short videos with a frame counter, colour bars and a checkerboard border. They show whether a player displays the picture correctly and loops gaplessly. |
| `packages/branding` | The project's colour palette and design sketches. |
| `packages/website` | The project site, <https://dex.ars.is>. |
| `packages/pi-gen` | Raspberry Pi's official tool for building OS images in stages; dexOS is a pi-gen build. |
| `packages/pi_video_looper` | Adafruit's Python video-looping framework. |

The player, the OS image and the website live in this repository, so one commit changes the player,
its packaging and the OS image together. The other four — `branding`, `example-content`, `pi-gen`
and `pi_video_looper` — are git submodules, pointers to separate repositories. Clone the
repository, then fill the submodules in:

```sh
git clone https://github.com/KTE/dex.git
cd dex
git submodule update --init
```

## Guides

Read these to build a player and keep it running. They assume you can use a terminal, and nothing
about video or Linux.

1. [What dexd is](docs/guides/what-dexd-is.md) — what the player does and what it needs.
2. [Build a dex card](docs/guides/build-player-card.md) — from a blank SD card to a booted player.
3. [Prepare your video](docs/guides/prepare-video.md) — turning the video you exported into the `.265` file and sidecar dexd accepts.
4. [Configure the exhibit](docs/guides/configure-exhibit.md) — the one file that names the video, the display mode and the connector.
5. [Run, check, troubleshoot](docs/guides/run-check-troubleshoot.md) — starting the player, reading the system log, and going from a symptom to a fix.
6. [Reference](docs/guides/reference.md) — config keys, sidecar keys, exit codes, file paths and every refusal message with its fix.

The package installs man pages for `dexd`, `dex-exhibit-apply` and `dex-wait-hdmi`. Build
`dex-sidecar` from source on the computer where you prepare the video; from the repository root,
read its page with `man ./packages/dexd/deploy/man/dex-sidecar.1`.

## Design documents

Read these to change the player. They assume a Linux or Rust developer who has not seen the
project.

| Page | Subject |
|---|---|
| [Architecture](docs/design/architecture.md) | The layers from Rust down to the display. |
| [The endless stream](docs/design/endless-stream.md) | How playback repeats without reaching the end of the file. |
| [Startup checks](docs/design/startup-checks.md) | What dexd verifies before it plays, and the exit codes. |
| [Failure handling](docs/design/failure-handling.md) | How a running player detects that it stopped showing pictures, and what it does then. |
| [The systemd unit](docs/design/service-unit.md) | Every setting in `dexd.service` and the scripts around it. |
| [Asset binding](docs/design/sidecar.md) | The sidecar file that records the video's frame rate and checksum, and the check that reads it. |
| [Exhibit config](docs/design/exhibit-config.md) | The per-installation file: grammar, refusals and the kernel command line `dex-exhibit-apply` derives from it. |
| [Packaging](docs/design/packaging.md) | What the `.deb` contains, and where it installs. |
| [Continuous integration](docs/design/ci.md) | What the workflow builds and asserts. |
| [Building and testing dexd](docs/design/development.md) | Build commands, test layers and their safety rules. |
| [Raspberry Pi media capability](docs/design/pi-capability.md) | What each Raspberry Pi generation can decode and display. |
| [Roadmap](docs/design/roadmap.md) | What is decided but not built, and what is out of scope. |
| [Measurement record](docs/design/measurements.md) | Every number the documentation relies on, and how it was established. |

## Vocabulary

[docs/glossary.md](docs/glossary.md) is the term list for this repository. Entries marked *user*
are the technical words the guides use without explaining them; entries marked *developer* are used
only in the pages under `docs/design/`. A word in neither list is plain English or is explained
where it is used. The writing rules are in [AGENTS.md](AGENTS.md).

## Licence

dexd's source, packaging and documentation are under the MIT-0 licence. The project's content —
test cards, video masters, branding — is under CC0-1.0. Both allow any use, with no attribution and no
conditions. Because the installed package links Debian's mpv library, the binary you install ships
under GPL-3+ — see [Packaging](docs/design/packaging.md). The submodules that point at other
projects, `pi-gen` and `pi_video_looper`, carry their own upstream licences, which the lines above
do not cover.
