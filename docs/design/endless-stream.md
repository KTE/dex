# Why an endless stream

dexd does not ask mpv to loop a file. It registers its own stream protocol and hands mpv a byte stream that never reports end-of-file: after the last byte of the video comes byte 0 again, forever. mpv plays one file, once, and never finishes it.

The reason is measured. Every looping mechanism on offer — in mpv, and in ffmpeg's direct-to-DRM output — holds a frame on screen every loop. mpv's mechanisms hold that frame at the loop point, because each makes the decoder re-enter the file, by a seek or by opening it again; ffmpeg's direct-to-DRM output holds one at the video's keyframes whether or not it re-enters. The endless stream removes the re-entry, and mpv is the decoder that takes a keyframe without stalling.

## The held frame at the loop point

With `--loop-file=inf` on the 3-second, 30 fps test video (90 frames), the last frame of the loop stays on screen for 83.3 ms against 33.3 ms for every other frame — an extra 50 ms, one and a half frame times, once per loop. A nominally 3.000-second video therefore repeats every 3.050 seconds. The held frame is always the same one, the video's last, in ten of ten loops with nothing skipped; a later, longer run held it in 61 of 61 loops across 11302 captured frames. Measured on a Raspberry Pi 4 against a capture of its 1920×1080, 60 Hz output, which samples each 30 fps source frame twice, so a held frame is directly observable; [measurements.md](measurements.md) holds the conditions.

Nothing is dropped. Every drop counter mpv reports reads zero through such a run, which is why the defect is invisible to the player's own statistics and shows only in a capture of the HDMI output.

## The seek as the cause

A file that reached the same point without seeking held nothing, which locates the defect in the seek. Three configurations were run on one setup: `--loop-file=inf`, which seeks at the end of the file; `--ab-loop-a=0 --ab-loop-b=2.99`, which seeks before the end; and a file holding the video concatenated twelve times, played straight through with no seek at all. The two seeking configurations each held the last frame seven times over seven loop points. The concatenated file held nothing: 597 displayed frames, each occupying two captures.

That `--ab-loop` behaves identically rules out mpv's end-of-file handling as the explanation — both configurations seek, and both keep the last frame on screen. At the loop point mpv seeks and must decode a fresh IDR, and the frame already on screen stays there while that happens.

Two consequences follow. The Raspberry Pi 4 presents 30 fps content with correct frame timing using the same file, decoder and output path, so the hardware is not the limit. And seek-based looping is the defect, so a fix has to decode the loop's start before playback reaches the end, or have the next pass already decoding before the current one ends.

## What each looping mechanism costs

Every player that re-enters the file stalls, and the stall lands wherever that player does its work — an end-of-file seek, a seek before the end, or a file open. ffmpeg's direct-to-DRM output is the exception: its stalls fall at the video's keyframes, and they stay there when the end of the file is taken away. The figures below were measured on a Raspberry Pi 4 with the same capture setup, 1920×1080, 60 Hz output; [measurements.md](measurements.md) holds the conditions.

| Mechanism | Where the held frame lands | Held frame |
|---|---|---|
| `--loop-file=inf` | at its end-of-file seek | 83 ms, once per loop |
| `--ab-loop-a` / `--ab-loop-b` | at its seek before the end | 83 ms, once per loop |
| `--playlist` with `--prefetch-playlist=yes` | at its open of the next entry | 117–133 ms, once per loop |
| `ffmpeg -stream_loop -1` into its direct-to-DRM output, `vout_drm` | at each keyframe, independent of the loop | 217 ms, three per loop |
| a concatenated file, one continuous decode | nowhere | none, across seven loop points |

`--prefetch-playlist` exists to make playlist transitions gapless and makes the stall worse. The ffmpeg row was run on the expectation that its decode headroom — it is the fastest raw decoder measured, 1.92× realtime (see vout_drm in the glossary and [measurements.md](measurements.md)) — would absorb the transition; it did not, and its three stalls per loop land near the video's keyframe boundaries, about a second apart, rather than at the loop point.

The two players fail in different places, and only mpv on an endless stream avoids both. mpv decodes an IDR mid-stream without stalling but stalls 83 ms on loop re-entry. ffmpeg's direct-to-DRM output stalls 67–217 ms on IDR frames, three per loop, but is clean at the loop point when fed the same endless stream. mpv on the endless stream has neither.

## What the endless stream delivers

Feeding mpv a stream that never ends removes the held frame entirely. Same Raspberry Pi 4, same video, same player, same display mode, only the feed differing: at 3840×2160, 30 fps the seeking configuration held its last frame ten times across nineteen loop points (the 4K capture samples below the display rate, so it catches only some; see capture deficit in the glossary), and the endless stream held nothing across nineteen. Confirmed at 1920×1080, 60 Hz output, where the capture samples every source frame twice and the answer is deterministic: nothing held across eight loop points. Measured; [measurements.md](measurements.md) holds the conditions.

## Why byte 0 after the last byte is not a seek

Concatenation worked because the decoder never re-initialises, not because the file was long. The video begins with the parameter sets followed by an IDR, and its GOPs are closed, so delivering byte 0 straight after the last byte hands the decoder another IDR, an ordinary mid-stream event that costs nothing. Annex-B HEVC concatenates at the bitstream level, so repeating a file's bytes produces a valid stream of any length.

The video is small enough to repeat from memory: the 3-second test video is 1.3 MB at 1920×1080 and 14.8 MB at 3840×2160. dexd reads it once at startup and keeps the bytes in memory — the payload — for the life of the process, which takes the filesystem out of the path entirely: no re-open, no page-cache dependency, no read stalling at the loop point.

## What the video must be

The video must be a raw Annex-B HEVC elementary stream — a .265 file, taken out of its video container once when the video is prepared — and it must have a closed GOP with an IDR at frame 0. An open GOP would make frame 0 depend on pictures that no longer exist when byte 0 comes round again, manufacturing a defect — a stall or a glitch — silently, at every loop point, roughly 29000 times a day for a 3-second video. The requirement holds even for content that does not visibly loop, because byte 0 is re-presented either way.

dexd enforces it at startup: the asset check reads the head of the stream and requires the parameter sets to appear before the first slice, and that slice to be an IDR. A CRA keyframe is refused, and the message names it as an open GOP. A stream with no Annex-B start code at all is refused as not being a raw HEVC stream. The checks, their messages and the exit-code contract are in [startup-checks.md](startup-checks.md); the checksum that binds the bytes to the prepared video is in [sidecar.md](sidecar.md).

A raw stream also carries no timestamps, so the frame rate travels beside the video: dexd passes the sidecar's frame-rate string to mpv's `container-fps-override` and, in its default option set, turns off `correct-pts`. Without both, mpv guesses a rate and plays at the wrong speed with every metric nominal. Preparing the video is where resolution, frame rate, GOP structure and keyframe placement are all normalised — see [../guides/prepare-video.md](../guides/prepare-video.md).

## The shell pipeline that proved the endless stream

`while true; do cat loop.265; done | mpv -`, with `--no-correct-pts --container-fps-override=30` and the Raspberry Pi display options listed in [architecture.md](architecture.md), is the endless stream in one line, and it works: zero held frames across nineteen loop points at 3840×2160, 30 fps, with flat memory over 3.5 hours (measured). It is not shippable for two reasons. It spawns a process per loop — roughly 29000 a day for a 3-second video, derived from a day divided by the loop length. And if mpv ever exits, `cat` is killed by the broken pipe and the shell loop busy-loops, using a full CPU core: a player that goes from a stopped video to a processor core at full load with no further warning.

## The loop:// stream

dexd does in one process what the pipeline did with a shell loop, `cat` and mpv. It registers a `loop://` protocol with libmpv through `mpv_stream_cb_add_ro` before initialising the library, so the protocol exists by the time playback is requested, then plays the URL `loop://endless`. The open callback ignores the URL — the video is fixed at startup, so there is nothing to parse and nothing that can fail there.

The read callback never returns 0. To mpv a read of 0 bytes is final end-of-file, the one event this player exists to prevent, so instead of reporting the end of the buffer the callback wraps its offset back to 0 and keeps copying. The reader's position lives in the per-stream state mpv hands back on every call, so it carries across calls instead of resetting on each one.

The seek callback reports the stream unseekable and the size callback reports the size unknown, both with `MPV_ERROR_UNSUPPORTED` (-18), as a pipe does. An mpv that believes it can seek will seek, and seeking is the operation that costs the frame; refusing leaves "keep reading forwards" as the only available behaviour. Reporting the payload length would let mpv compute a duration and a progress position for a stream that has neither, and invite it to treat the end of the buffer as the end of the media. `MPV_ERROR_UNSUPPORTED` is the documented sentinel for stream callbacks; `-1` also happens to work, but only because mpv 0.40 tests the sign of the return value.

Seeking is therefore gone by construction. That costs dexd nothing, since it only ever loops, and it rules this configuration out as a general-purpose player.

## Loop-position arithmetic

All of the decision-making lives in one pure function, `next_chunk(len, pos, want)`, which the read callback surrounds with the one memory copy the pure function cannot perform. Given the payload length, the reader's position and the requested byte count, it answers with an offset to copy from, a byte count and the position afterwards:

```
start    = pos if pos < len else 0
n        = min(want, len - start)
next_pos = 0 if start + n == len else start + n
```

Three properties follow, and each is locked in by a test.

- A short read is legal, per mpv's stream-callback header, so the loop point is never stitched across a single call: a request larger than the bytes remaining is answered with the tail now and the head on the next call.
- The position returns to 0 eagerly: when a copy reaches the end of the payload, `next_pos` is 0 and never the payload length, so between calls the position is always below the payload length.
- No answer is ever zero bytes. `next_chunk` returns nothing at all — which the caller turns into an mpv error, never 0 — in the two cases where it can produce no bytes without lying: a request for zero bytes, or an empty payload.

Together they are what the read callback's unsafe copy rests on: at least one byte, never more than mpv asked for, a source range that never leaves the payload, and a destination buffer that cannot overlap it.

Neither case, a zero-byte request or an empty payload, can happen in service: mpv 0.40 guards a zero-length request before calling in, and an empty video is refused at startup. The read callback's error-return branch is kept as a sentinel so dexd's own diagnostics can tell "asked for nothing" apart from "ran out of things to give"; a negative return would end the stream just as 0 does, since mpv maps any value at or below 0 to end-of-file. If it ever did fire on a later mpv, the result is an ordinary `END_FILE`, a fatal exit and a restart by systemd — not a held frame ([failure-handling.md](failure-handling.md)).

The zero-byte property is the one a change could break invisibly. A callback that returned 0 for a zero-length request would leave every test in the crate passing except the one asserting an mpv error, and no run on hardware could catch it, because mpv never issues a zero-length read.

The request size arrives from mpv as a 64-bit count and is converted with `usize::try_from`, falling back to the largest `usize`. On a 32-bit target a plain cast would turn a request that is an exact multiple of 2^32 into zero bytes and so into a spurious end-of-file; saturating can only shrink a request, and a short read is legal. dexd's target, 64-bit Raspberry Pi OS on a Pi 4, always succeeds at the conversion, so the branch is not exercised on target hardware (not tested); the test locks the contract against a plain cast.

The property that matters most is tested directly: driving `next_chunk` repeatedly reproduces the payload repeated endlessly, byte for byte, over six request schedules against a 997-byte payload, a prime length — one byte at a time, the payload length, one short of it, one past it, 4096 bytes and a 2000-step pseudo-random schedule of sizes 1 to 300 from a fixed generator. Splitting the arithmetic out this way is what lets it be tested without libmpv and without a Raspberry Pi; the split is described in [architecture.md](architecture.md).

## The loop counter and the read-ahead

The heartbeat's `loops=` field counts completed passes over the payload. The read callback increments it whenever a copy lands back at position 0, on the demuxer's thread; the heartbeat reads it on the supervisor thread with relaxed ordering, a monotonic diagnostic rather than a synchronisation point. It counts demuxer passes, which run about one second ahead of the picture, so a heartbeat line names a slightly later loop than the screen shows. The heartbeat's other fields are in [failure-handling.md](failure-handling.md).

That one second is the read-ahead: `demuxer-readahead-secs` is set to 1.0, which is the prefetch depth across the loop point where the gapless claim is decided. `demuxer-max-bytes` is set to 64 MiB as a second safeguard — mpv enables its stream cache by default only for streams it classifies as network, which a stream-callback stream is not, so the flat memory measured over 3.5 hours comes from the read-ahead setting.

## Players considered before this design

A survey of Raspberry Pi video players in 2024 found no player that looped gaplessly on a Raspberry Pi OS newer than buster; hello_video, on buster, did. omxplayer, the general-purpose alternative of the day, leaves about 100 ms of black between plays (assumed: no instrument recorded), which for a looping artwork is the defect that matters. The players below were tried by eye on a Raspberry Pi Zero 2 W (assumed: no instrument). `cvlc`, the VideoLAN player's command-line form, was not gapless, and gapless behaviour was expected only in its then-unreleased version 4. mplayer, another general-purpose player, did not work as installed. hello_drmprime, run with a repeat count, was not gapless. A GStreamer attempt stuck on the first frame, was not gapless, and dropped the display between plays. Cog, the minimal launcher for the embedded WebKit engine, played an H.264 test card gapless with occasional hiccups but stuttered on HEVC and logged a decoder warning about frames left undrained. pivid built and looped a 1-second video on a Raspberry Pi 4 (whether gaplessly was not recorded); it ran on a Raspberry Pi Zero 2 W only unstably, and its 32-bit builds later failed (see pivid in the glossary). Locking the output to 1920×1080, 30 Hz changed nothing for cvlc, mplayer or Cog, which ruled out 4K output as the cause.

What remained was hello_video, which loops seamlessly when the same video is played more than once but plays raw H.264 only and needs Debian buster. The dex project's earlier player image, dexOS, is built on it, trading away audio and container-format support for a loop with no visible break, and gapless looping on anything newer than buster stayed an open problem. The endless stream closes it, and makes a general-purpose player usable in its place (derived): with a preparation step, the raw elementary stream that reads as hello_video's limitation is simply the output format. Where that leaves dexOS is in [roadmap.md](roadmap.md).
