# The endless stream

This page explains how dexd repeats a video with no visible break at the loop point, what the method requires of the video file and why it uses none of the looping options mpv and ffmpeg offer. It is written for a developer reading the player's source.

Measured numbers on this page come from a Raspberry Pi 4 with the 3-second test video; the conditions are in [measurements.md](measurements.md).

## Method

dexd gives mpv the video as one byte stream that never reports end of file, so the decoder neither seeks nor restarts.

The decoder never re-initialises. The video's first picture is an IDR at the head of a closed GOP, so presenting byte 0 straight after the last byte is an ordinary mid-stream keyframe, and the decoder does no seek. Annex-B HEVC concatenates at the byte level, so the repeated bytes are a valid stream.

## Requirements on the video

- A raw Annex-B elementary stream, not MP4. ffmpeg writes it directly with `-f hevc`; to take an existing HEVC stream out of a video container, run a one-off `-c:v copy -bsf:v hevc_mp4toannexb` — see [prepare-video.md](../guides/prepare-video.md).
- A closed GOP whose first picture is an IDR, preceded by the VPS, SPS and PPS parameter sets.
- A frame rate stored beside the file, because a raw stream carries no timestamps. dexd passes the sidecar's rate to `--container-fps-override` together with `--no-correct-pts`; without both, mpv guesses a rate and plays at the wrong speed.

An open GOP would make the first picture depend on pictures that no longer exist when the stream returns to byte 0, producing a visible glitch at every loop point — about 29,000 a day for a 3-second video (derived).

dexd checks the parameter sets and the first slice at startup and refuses a video that does not begin with an IDR. That check and its exit code are in [startup-checks.md](startup-checks.md), the frame rate and the checksum in [sidecar.md](sidecar.md).

## loop:// stream

dexd registers the `loop://` scheme with libmpv before mpv initialises, then plays the URL `loop://endless`. At startup it reads the whole video into memory once, as the payload: 1.3 MB at 1920×1080 and 14.8 MB at 3840×2160 for the 3-second test video, so no file I/O happens at the loop point.

When mpv opens the URL, dexd's open callback creates a per-stream state with the payload and the position, and installs four callbacks. mpv passes the state back on every call.

- `read_fn` copies the requested bytes and moves the position back to byte 0 when it reaches the end of the payload, instead of returning 0. Returning 0 would tell mpv the file has ended.
- `seek_fn` returns `MPV_ERROR_UNSUPPORTED` (-18), mpv's documented "not supported" return for a stream callback, so the stream behaves like a pipe. If the stream reported itself seekable, mpv would seek at the loop point, and that seek leaves the last frame on screen too long — see [Loop-point stalls](#loop-point-stalls).
- `size_fn` returns the same error. A length would let mpv compute a duration and a playback position for a stream that has neither, and mpv could treat the end of the payload as the end of the media (not tested).

The stream cannot seek, so mpv offers no scrubbing. That suits a player that only ever loops and rules this configuration out as a general-purpose one.

## Loop position arithmetic

`chunk.rs` computes the position: `next_chunk(len, pos, want) -> Option<Chunk>`, tested without libmpv. `len` is the payload length, `pos` the reader's position and `want` the bytes mpv asked for. `Chunk` carries the offset to copy from, the byte count and the position afterwards. The read callback is a thin unsafe shell that performs the copy, under the bounds `next_chunk` guarantees: at least one byte, never more than mpv asked for, inside the payload and never overlapping the destination.

- The byte count is `min(want, len - start)`, so `next_chunk` answers a request larger than the bytes remaining with a short read. mpv's `stream_cb.h` permits that, so no single call spans the return to byte 0: the tail comes now, the head on the next call.
- The position after the call is always below `len`. The return to byte 0 happens in the call that reaches the end, never in the following one, so the bounds the unsafe copy relies on hold between calls.
- `None` means `want == 0` or an empty payload; the caller turns it into an mpv error, never 0. mpv 0.40's `stream.c` rejects zero-length reads before invoking the callback, so the branch never runs.
- `clamp_want` converts mpv's `u64` request size with `usize::try_from(nbytes).unwrap_or(usize::MAX)`. `as usize` would truncate a request of an exact multiple of 2^32 to zero bytes on a 32-bit target and end the stream. On the aarch64 target the conversion always succeeds, so the test states the contract rather than exercising it (not tested).

One test drives `next_chunk` over a 997-byte payload with six request-size schedules: one byte at a time, the payload length itself, one short of it, one past it, 4096 bytes and a deterministic pseudo-random schedule of 2000 sizes. It asserts the output equals the payload repeated endlessly, byte for byte. Further tests lock in the boundary cases: the point where the position returns to byte 0, a payload smaller than the request and the zero-length request that must not become a zero-byte answer.

## Read-ahead

dexd sets mpv's `--demuxer-readahead-secs=1.0`, the prefetch depth: one second of demuxed packets ahead of the decoder, across the loop point. mpv runs its larger cache only for streams flagged as network, and a stream-callback stream is not one. `--demuxer-readahead-secs` is therefore the bound that applies, and `--demuxer-max-bytes=64MiB` is a second safeguard.

dexd's resident memory stays flat at 221 MB over 20 s playing the 3840×2160, 30 fps test video (see [measurements.md](measurements.md)).

## Loop counter

The read callback increments a counter on the demux thread each time the position returns to byte 0; the supervisor thread reads it for the heartbeat, which prints it as `loops=` — see [reference.md](../guides/reference.md#system-log-lines). Relaxed ordering is enough for a monotonic diagnostic. The counter counts demuxer passes, which run about one second ahead of what is on screen.

## Loop-point stalls

With `--loop-file=inf`, mpv leaves the final frame on screen for 83 ms against 33 ms for every other frame. The extra 50 ms falls once per loop, so a 3.000 s video repeats every 3.050 s, which reads as a hesitation in smooth motion (assumed). The frame is the last frame of the loop every time: ten of ten loops in one run, with no frame skipped, and 61 of 61 in a longer one. mpv's drop counters stay at zero, so nothing is dropped.

The seek causes the held frame. `--ab-loop-a`/`--ab-loop-b` seeks before the end of the file and leaves the same frame on screen just as often, which rules out mpv's end-of-file path specifically. A file containing the same video twelve times over, decoded straight through with no seek, showed no held frame: 597 distinct frames seen at the HDMI output, each for one frame time, across seven loop points. At the loop point mpv seeks and decodes a fresh IDR, and the picture already on screen stays there while that happens (derived).

The continuous run played the same encoded video through the same decoder and output path, so the Raspberry Pi 4 presents 30 fps content with correct frame timing. Seek-based looping causes the defect, and a gapless loop must reach the next repeat's first frame without a seek.

Each mechanism stalls where it does its work: at the seek, at the playlist open or on the keyframe.

| Mechanism | Where it does its work | Held frame | Position |
|---|---|---|---|
| mpv `--loop-file=inf` | seek at end of file | 83 ms, once per loop | last frame of the loop |
| mpv `--ab-loop-a` / `--ab-loop-b` | seek before end of file | 83 ms, once per loop | last frame of the loop |
| mpv `--playlist` with `--prefetch-playlist=yes` | opens the next playlist entry | 117–133 ms, once per loop | last frame of the loop |
| ffmpeg `-stream_loop -1 -f vout_drm` | re-enters the file | 67–217 ms, three times per loop | at keyframes, not at the loop point |
| one continuous decode of concatenated content | none | none | — |

On a raw .265 file, `--loop-file=inf` freezes on the last frame instead of looping (see [measurements.md](measurements.md)).

The endless stream was measured against `--loop-file=inf` on the same video, player and display mode, differing only in how the video reached mpv. It left no held frame: none across 8 loops at 1920×1080, 60 Hz output (30 fps content, so every frame is captured twice), and none across 19 loops at 3840×2160, 30 Hz. The seeking configuration held the last frame in 10 of those 19 loops.

At 3840×2160 the HDMI capture device used for these runs delivers about 27 fps against 30 fps content — the [capture deficit](../glossary.md). The 4K count therefore corroborates the oversampled 1920×1080 runs rather than standing on its own.

The endless-stream runs fed the bytes with the shell pipeline under [Alternatives](#alternatives). dexd's `loop://` callback emits the same bytes: its tests assert that `next_chunk` reproduces the video repeated endlessly, byte for byte.

The two players fail in different places. mpv decodes IDR frames without stalling and stalls only when it re-enters the file. ffmpeg's direct-to-DRM output (`vout_drm`) stalls on IDR frames instead, with or without an endless stream, so its stalls fall away from the loop point. mpv on a stream that never ends has neither stall.

## Alternatives

The project judged Cog, VLC, mplayer, hello_drmprime and a GStreamer 1.22 attempt on screen in 2024, on a Raspberry Pi Zero 2 W with H.264 and H.265 test videos.

| Option | Outcome | Why not |
|---|---|---|
| `while true; do cat video.265; done \| mpv -` | gapless — the comparison under [Loop-point stalls](#loop-point-stalls); no memory growth over 3.5 h | spawns about 29,000 processes a day for a 3-second video and busy-loops on a full CPU core if mpv exits |
| hello_video | gapless, and the player of the legacy dexOS image | H.264 only, no audio |
| omxplayer | about 100 ms of black between plays (assumed) | removed from Raspberry Pi OS since bullseye (Debian 11) |
| pivid | purpose-built gapless player for installations | 32-bit builds failed, project dormant |
| Cog on WPE WebKit | gapless for H.264 with occasional hiccups | HEVC through the same path stutters |
| VLC | not gapless | gapless mode was expected only in VLC 4, unreleased at the time |
| hello_drmprime | not gapless | the zero-copy reference program, not a player |
| mplayer | did not play | did not work out of the box |
| a GStreamer 1.22 attempt | not gapless | stuck on the first frame and dropped the display between plays |
