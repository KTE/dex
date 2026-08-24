# Prepare your video

dexd plays one video, and it plays only a .265 file: HEVC pictures with no video container around them and no sound. Follow this page to turn the file the artist gave you — the [artist's supplied master](../glossary.md#artists-supplied-master), whatever format it arrived in — into that .265 file and its sidecar, before anything goes to a dex card.

You need ffmpeg with ffprobe, and Rust's build tool `cargo`, on the workstation where you prepare the video.

You also need `dex-sidecar`, which the player package does not install: prepare a video on a workstation, never on the device. Build it from the dex source ([building and testing](../design/development.md)):

```sh
git clone --no-recurse-submodules https://github.com/KTE/dex.git ~/dex
cd ~/dex/packages/dexd
cargo build --release --bin dex-sidecar
```

The binary lands in `target/release/dex-sidecar`; copy it onto your `PATH` or call it by that path.

## Requirements on the video

- A .265 file: one video stream, no video container, no audio track.
- A keyframe at the first frame, with no later picture referring back across it. dexd loops by feeding the file to the decoder without ever stopping, so the first byte must read as an ordinary keyframe each time round.
- A capped bitrate — see [Bitrate](#bitrate).
- A sidecar beside it. A .265 file does not record its frame rate; dexd takes the rate from `<video>.json` and refuses to start without it.

## Encoding a master

```sh
ffmpeg -i master.mov \
  -c:v libx265 -pix_fmt yuv420p \
  -crf 20 -maxrate 35M -bufsize 70M \
  -x265-params keyint=90:min-keyint=90:no-open-gop=1:scenecut=0 \
  -an -f hevc artwork.265
```

- `-pix_fmt yuv420p` writes 8-bit colour, which a Raspberry Pi 4 decodes in hardware.
- `-crf 20` sets the quality target; a lower number gives a better and larger file.
- `-maxrate 35M -bufsize 70M` cap the peaks.
- `keyint=90:min-keyint=90` place a keyframe every 90 frames — three seconds at 30 fps; use your own frame rate times the seconds you want.
- `no-open-gop=1` keeps each keyframe complete, so no picture after it refers to a picture before it.
- `scenecut=0` stops the encoder adding keyframes of its own at scene changes.
- `-an` drops the audio; `-f hevc` writes the .265 file directly.

**Important:** a .265 file records no rotation, so the picture is shown exactly as its pixels are stored. A master from a phone usually keeps its rotation in the video container rather than in its pixels, and the two routes here treat that differently: encoding applies the rotation, so the picture comes out upright with its width and height swapped, while the stream copy below leaves the pixels as they were stored. Whichever route you take, check the result before the video goes to a venue:

```
ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 artwork.265
```

The width and height it prints are the ones the artwork will be shown at.

## Files already in HEVC

When the master is HEVC inside an MP4 or MOV, take the pictures out without re-encoding:

```sh
ffmpeg -i card.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc artwork.265
```

The copy keeps whatever keyframe structure the file already has, so if dexd refuses the result, encode from the master with the command above.

## Frame rate

Use the rate the master has, and write a rate like 29.97 fps as the fraction it is: 30000/1001. The sidecar carries it in that form.

Going from 59.94 fps to 29.97 fps drops every second frame with nothing resampled, and roughly halves the bits needed for the same quality. Add `-r 30000/1001` to the encode command, after `-i master.mov`, when the artwork does not need the higher rate.

## Bitrate

Cap the bitrate: a fixed-quality encode can spend a great deal on the hardest few seconds, and one such passage stalls the decoder while the file's average stays modest. A fixed bitrate bounds the peak too, at the cost of spending the same bits on easy sections.

Measured on a Raspberry Pi 4, these encodes play; the conditions for each are in the [measurement record](../design/measurements.md):

| Video | Bitrate |
|---|---|
| 3840x2160, 30000/1001 fps | 25.3 Mbit/s |
| 3840x2160, 30 fps, grain-heavy test video | 39.7 Mbit/s |
| 2560x1440, 60 fps | 45 Mbit/s cap |

**Note:** a single community report on a Raspberry Pi 4 puts smooth 4K30 HEVC playback around 60 Mbit/s and choppy playback above roughly 80 Mbit/s. Read it as an order of magnitude; this project has measured no ceiling of its own.

Above about 40 Mbit/s an encode may also cross into a higher HEVC tier, a label in the file saying how demanding it is. When playback fails abruptly near that figure rather than getting worse by degrees, check the `profile`, `level` and `tier` line x265 prints while encoding.

When you cannot tell whether a bitrate is safe, encode a ladder — the same master at `-maxrate 20M`, `30M`, `40M`, `50M`, or at `-crf 18`, `20`, `22` under one cap.

Write a sidecar for each. Copy a candidate and its sidecar to `/opt/dex` on a Raspberry Pi, name it in the exhibit config's `asset:` line and restart dexd — see [Installing on the player](#installing-on-the-player). Judge each candidate against a file you already know plays correctly, not against an absolute number. Watch `frame-drops=` and `vo-delayed=` in the heartbeat — see [run, check, troubleshoot](run-check-troubleshoot.md).

## The sidecar

`dex-sidecar write` reads the frame rate, hashes every byte of the video and writes `<video>.json` beside it:

```sh
dex-sidecar write artwork.265
```

```
note: frame rate 30000/1001 read from artwork.265's own headers. Pass --fps if you know the rate from somewhere else.
wrote artwork.265.json
{"fps":"30000/1001","sha256":"25d17945...","width":3840,"height":2160}
```

The rate comes from timing the encoder wrote into the stream. `dex-sidecar` uses a rate it reads there only when that rate falls between 1 and 1000 fps, and exits 2 when it can read none. Give the rate yourself:

```sh
dex-sidecar write artwork.265 --fps 30000/1001
```

**Important:** `dex-sidecar write` refuses a `--fps` that disagrees with the rate it read from the stream and exits 2. At the wrong rate the video plays too fast or too slow for as long as it runs, and nothing reports an error. Add `--force` when your value is the right one; `--force` also lets it replace a sidecar that already exists.

Do not write the sidecar by hand. `dex-sidecar write` reads its own output back with dexd's parser and re-checks the hash before it writes the file.

To confirm that a video and its sidecar still match after a copy:

```sh
dex-sidecar check artwork.265.json artwork.265
```

`dex-sidecar check` prints `OK` with the frame rate, checksum and picture size, and exits 1 when the video no longer matches its sidecar.

## Installing on the player

Copy both files — `artwork.265` and `artwork.265.json` — into `/opt/dex` on the dex card, and name the video in the exhibit config beside them:

```yaml
asset: artwork.265
```

`/opt/dex` is the assets directory on the player, reached over the network. The file name is yours to choose and several videos can sit there; the exhibit config names which one plays.

A new video plays from the next start: `sudo systemctl restart dexd`. See [configure the exhibit](configure-exhibit.md).

The package installs no video and no exhibit config: content changes per installation.

## Refusal messages

dexd checks the file before it shows anything, and writes the reason to the system log. A file that starts wrong would otherwise play and glitch once per loop — roughly 29,000 times a day for a three-second loop, with nothing reporting an error.

| Message | Fix |
|---|---|
| `no Annex-B start code found` | The file is still in a video container; extract it with the copy command above. |
| `leading keyframe is CRA (open GOP)` | Encode again with `no-open-gop=1`. |
| `first slice NAL is type N, not an IDR` | The file does not start on a keyframe; encode again from the master. |
| `first slice appears before parameter sets` | The file's header is incomplete; encode again from the master. |
| `parameter sets but no slice found in the asset` | The file has a header but no picture data; encode again from the master. |
| `no sidecar found` | Run `dex-sidecar write` and copy the `.json` alongside. |
| `asset does not match its sidecar` | The copy is stale or incomplete; copy again, then `dex-sidecar check`. |

Every message with its exit code is in the [reference](reference.md); symptoms that appear while playing are in [run, check, troubleshoot](run-check-troubleshoot.md).

## Test cards

The example-content package provides short test cards — a frame counter, three rotating hands, colour bars, a checkerboard border and resolution wedges of fine converging lines. They exist to check that a player shows the picture correctly and loops gaplessly; how a test card becomes a video is documented in that package.

## Reference

Manual page: `dexd`(1); `dex-sidecar` prints its options and exit codes when run with no arguments · [what dexd is](what-dexd-is.md) · [glossary](../glossary.md) · [why an endless stream](../design/endless-stream.md)
