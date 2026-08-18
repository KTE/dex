# Why an endless stream

dexd does not use mpv's loop options. It gives mpv one stream that has no end: the read callback — the function libmpv calls when it needs more bytes — jumps back to byte 0 instead of reporting end-of-file. The decoder keeps running and no frame is held at the loop point. mpv's three looping mechanisms each re-enter the file — seek back into it or reopen it — and leave the last frame on screen for 83 to 133 ms, against the 33 ms one frame lasts at 30 fps; ffmpeg's direct-to-DRM output stalls at keyframes however it is fed.

## The defect at the loop point

mpv's `--loop-file=inf` shows the video's last frame for 83.3 ms instead of 33.3 ms, once per loop — measured on a Raspberry Pi 4 at 1920×1080, 60 Hz output, with a 3-second, 30 fps test video recorded by an HDMI capture device at twice the frame rate (see the [measurement record](measurements.md)). The video has 90 frames, and only the last one is held.

The extra 50 ms per loop makes the real loop period 3.050 s against a nominal 3.000 s. A hesitation of that length in smooth motion is visible as a stutter (assumed).

The held frame appears in every loop of a 10-loop run (no frame skipped) and of a 61-loop run.

## The cause: a seek at every loop

The seek causes the held frame, not mpv's end-of-file handling. Three configurations ran on one setup, with the same video, decoder and output path:

- `--loop-file=inf`, which seeks at the end of the file: 7 held frames in 7 loops.
- `--ab-loop-a=0 --ab-loop-b=2.99`, which seeks before the end of the file: 7 held frames in 7 loops.
- a file containing twelve copies of the video, decoded continuously with no seek: 597 frames, no held frame, 7 loop points clean.

After the seek the decoder must decode a fresh IDR (a keyframe) before it can show a new picture; the frame already on screen stays until then. The decoder drops no frame, which is why mpv's drop counters read zero throughout (derived).

The continuous run proves a Raspberry Pi 4 presents 30 fps content with correct frame timing. And a fix has to remove the seek: decode the start of the next pass early, or keep the next pass already decoding.

## What each looping mechanism costs

Every mechanism that re-enters the file stalls, measured on the same Raspberry Pi 4 and test video.

| Mechanism | Work at the loop point | Held-frame duration | Held frames per loop | Where the held frames appear |
|---|---|---|---|---|
| mpv `--loop-file=inf` | seek at end of file | 83 ms | 1 | the loop point |
| mpv `--ab-loop-a` / `--ab-loop-b` | seek before end of file | 83 ms | 1 | the loop point |
| mpv `--playlist` with `--prefetch-playlist=yes`, mpv's gapless-transition feature | opens the next playlist entry | 117–133 ms | 1 | the loop point |
| ffmpeg `-stream_loop -1 -f vout_drm` | not recorded | 217 ms | 3 | keyframe boundaries, about 1 s apart |
| twelve copies in one file, single continuous decode | none | — | 0 | — |

Feeding ffmpeg an endless stream still leaves three stalls per loop on the keyframes, 67–217 ms.

## The endless stream

Continuous decoding works because the decoder never re-initialises, not because the file is long. The test video's GOPs are closed and its first picture is an IDR, so presenting byte 0 straight after the last byte is an ordinary mid-stream keyframe rather than a seek. Annex-B HEVC concatenates at the byte level, so one copy of the file, repeated, is the whole stream: the 3-second test video is 1.3 MB at 1920×1080 and 14.8 MB at 3840×2160.

A side-by-side comparison of `--loop-file=inf` against the endless stream changed only how mpv was fed the video: same video, player and display mode. At 3840×2160 and 30 fps, where the HDMI capture device cannot sample each frame twice (see the [measurement record](measurements.md)), `--loop-file=inf` showed a held frame in 10 of 19 loops and the endless stream showed none in 19. At 1920×1080 at 60 Hz output, where every source frame is sampled twice and the result is deterministic rather than statistical, the endless stream showed none in 8 loops.

The endless stream in that comparison was a shell pipeline, `while true; do cat video.265; done | mpv -`, with dexd's decode and output options. mpv's memory stayed flat over 3.5 hours of the pipeline running (measured).

## What the video must be

The endless stream requires a raw Annex-B .265 file whose GOPs are closed and whose first picture is an IDR; with an open GOP the first pictures would depend on pictures the decoder no longer has, and a frame would be held at every loop point (not tested).

Two requirements follow for the rest of the system.

- The frame rate is stored in the sidecar next to the video. An elementary stream carries no timestamps, so dexd passes `--no-correct-pts` together with `--container-fps-override` and the rate from the sidecar. The prepare-video recipe sets the frame rate and GOP structure — see [Asset binding](sidecar.md) and [Prepare your video](../guides/prepare-video.md).
- Seeking is unavailable by construction. dexd reports the stream as unseekable, so scrubbing and resuming at a position are not possible; that suits a player which only loops and rules out the endless stream for a general-purpose player.

dexd verifies the start of the asset before it starts mpv — parameter sets at the head of the file, an IDR as the first picture — and the sidecar's checksum over every byte; it refuses to start otherwise. See [Startup checks](startup-checks.md).

## The loop:// stream

dexd registers a `loop://` URL scheme with libmpv and plays `loop://endless`. dexd reads the asset into memory once at startup — the payload — and the read callback copies out of it. The read position lives in the per-stream state that mpv hands back on every call, so successive reads continue where the last one stopped.

| Callback | Behaviour |
|---|---|
| read | Copies the next bytes and sets the position back to 0 at the end of the payload. Never returns 0, because to mpv a read that returns 0 is final end-of-file. |
| seek | Returns `MPV_ERROR_UNSUPPORTED` (-18), like a pipe. With an unseekable stream mpv can only read forwards, and seeking is the operation that produces the held frame. |
| size | Returns `MPV_ERROR_UNSUPPORTED` (-18). A reported size would let mpv compute a duration and a playback position for a stream that has neither, and mpv could then treat the end of the buffer as the end of the media (not tested). |

## Loop-position arithmetic

The read callback is a thin unsafe shell over one pure function, `next_chunk(len, pos, want)` in `chunk.rs`, tested without libmpv. It returns a chunk `{start, n, next_pos}`: where in the payload to copy from, how many bytes, and the reader position after the copy. Its contract:

- It returns no chunk only for a zero-length request or an empty payload. The caller turns that into an mpv error, never 0.
- The copy starts at `pos`, or at 0 when `pos` has reached the end of the payload; the chunk length is `min(want, len − start)`, so a request larger than the rest of the payload gets a short read. A short read is legal per mpv's `stream_cb.h`, so one call never returns bytes from both the end and the start of the payload: the tail comes now, the head on the next call.
- The position returns to 0 as soon as a copy reaches the end of the payload, so `next_pos` is always below the payload length between calls.
- The request-size clamp (`clamp_want`) converts mpv's `u64` request size with `usize::try_from`, saturating at `usize::MAX`. Truncating with `as usize` would turn a request that is a multiple of 2^32 into a zero-byte request on a 32-bit target, and so into a spurious end-of-file. dexd is packaged for arm64 only, so that fallback never triggers on target hardware; a test enforces the contract.

The copy relies on three guarantees from `next_chunk` — n ≥ 1, n ≤ the request, start + n ≤ the payload length — and, because mpv's buffer is not the payload, the source and destination never overlap.

Driving `next_chunk` repeatedly reproduces the endlessly repeated payload, byte for byte, over six request schedules against a 997-byte payload: one byte at a time, the payload length itself, one short of it, one past it, 4096 bytes, and a 2000-step pseudo-random schedule. A separate test asserts that the read callback answers a zero-length request with an mpv error rather than 0. Boundary tests lock in the answers at the exact end of the payload and for a payload smaller than one request.

The read callback's zero-length branch is unreachable on mpv 0.40, which never issues a read of 0 bytes (a guard in mpv 0.40's `stream.c`). The branch stays so that dexd never answers a request with 0; if a later mpv does call with 0, mpv ends the stream, dexd sees `END_FILE`, exits, and systemd restarts it (assumed).

## Loop count and read-ahead

The heartbeat prints `loops=`, the number of completed passes over the payload. The read callback increments it on the demux thread whenever the position returns to 0, and the supervisor thread reads it. `loops=` counts demuxer passes, which run about 1 second ahead of the picture on screen.

That second is the read-ahead depth. dexd sets `demuxer-readahead-secs` to 1.0: one second of packets read ahead across the loop point. mpv enables its cache only for streams flagged as network, and a stream-callback stream is not one, so this setting governs read-ahead. `demuxer-max-bytes` (64 MiB) caps the demuxer's memory as a second safeguard.

## Other options

The project tried these players on a Raspberry Pi Zero 2 W with its H.264 test card, with no capture instrument: each result is as observed on screen (derived). Where a player was tested differently, the line says so.

- hello_video — gapless, in upstream's own description; the player the old dexOS image is built on; plays raw H.264 only and needs buster.
- omxplayer — about 100 ms of black between plays (assumed; no method recorded).
- pivid — looped a 1-second video on a Raspberry Pi 4; unstable on the Pi Zero 2 W, and its 32-bit build failed.
- VLC (`cvlc`) — not gapless; gapless playback was expected only from a later major version.
- mplayer — did not run without further work.
- hello_drmprime — not gapless.
- GStreamer — a 1.22 attempt stuck on the first frame, not gapless, and dropped the display between plays; a later `gst-play-1.0 --use-playbin3 … --videosink kmssink --gapless` try has no recorded outcome.
- Cog, a launcher for an embedded browser engine — gapless for H.264 with occasional hiccups; H.265 through the same path stuttered.

The shell pipeline from the side-by-side comparison in "The endless stream" is not shipped: it starts one process per loop, about 29,000 a day for a 3-second video (derived), and if mpv exits, the shell loop busy-loops on a full CPU core.
