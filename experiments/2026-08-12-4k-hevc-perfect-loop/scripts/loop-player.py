#!/usr/bin/env python3
"""Gapless HEVC looper: libmpv fed by an endless in-process byte stream.

WHY THIS EXISTS
---------------
Every mpv looping mechanism stalls at the wrap, because each one makes the
decoder re-enter the file (measured 2026-08-13/14 on a Pi 4 at 4K30 and
1080p60, with an HDMI capture card):

    --loop-file=inf                 83 ms hold of the last frame, every loop
    --ab-loop-a/b                   83 ms, identical
    --playlist + --prefetch         117-133 ms, worse
    ffmpeg -stream_loop + vout_drm  67-217 ms, three per loop, at IDR frames
    --loop-file=inf on raw .265     freezes on the last frame (no index to seek)

The only configuration with ZERO held frames is one where the decoder never
reaches EOF at all. That works because the asset has a closed GOP with an IDR
at frame 0, so presenting byte 0 straight after the last byte is an ordinary
mid-stream IDR, not a seek.

`while true; do cat loop.265; done | mpv -` proves the point but is not
shippable: it spawns a process per loop (~29k/day for a 3 s loop), and if mpv
ever exits, `cat` dies on SIGPIPE and the shell loop spins hot forever.

This does the same thing inside one process. python-mpv's `python_stream`
takes a GENERATOR, so an infinite generator is a legitimate stream that simply
never ends -- no pipe, no subprocesses, no busy-loop failure mode, and mpv's
own lifecycle governs everything.

REQUIREMENTS
------------
* A raw Annex-B HEVC elementary stream, not MP4:
      ffmpeg -i card.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc loop.265
* --no-correct-pts plus the real frame rate: a raw stream carries no
  timestamps, so mpv must be told the rate. Frame rate becomes ingest metadata.

STATUS 2026-08-14: DISPLAYS CORRECTLY, BUT RUNS AT ~0.6x REALTIME.
------------------------------------------------------------------
CORRECTION to an earlier note in this file: the claim that libmpv "does not
scan out" was WRONG, and was an artifact of how the process was launched.
Backgrounding it over SSH killed it before every measurement, so the readings
described a corpse. Two further traps found while establishing that:

  * `pgrep -f loop-player` also matches the `timeout` wrapper, so a liveness
    check can report a running player that is really a shell.
  * mpv exits immediately unless stdin is redirected: run with `</dev/null`,
    or it sees EOF on stdin and quits.

Launched correctly (`nohup setsid ... </dev/null &`) libmpv works exactly as
the CLI does: 14 threads, 4 card1 + 2 renderD128 fds, DRM master `y`, both
planes bound (91 and 127 to pixelvalve-2), and real video on the HDMI capture
with zero decode failures.

THE REAL DEFECT is throughput. Measured over 900 captured frames (30 s):

    shell pipe   : 19 wraps, dwell {1: 1500}          -- 1.0x, seamless
    this script  :  6 wraps, dwell {1:309, 2:44, 3:21, 4:29, ... 12:2}

Six loops where the pipe manages nineteen: **~0.6x realtime**, with ~57 held
frames scattered across the WHOLE loop rather than concentrated at the wrap.
Scattered holds are a feed-starvation signature, not a seam -- the decoder is
being fed too slowly, so frames linger wherever the shortfall lands.

The difference between the two is only who supplies the bytes: `cat` in C, or
this generator in Python via a ctypes callback. That points at per-read
callback overhead and GIL contention rather than anything in mpv.

Worth trying before abandoning the approach: much larger CHUNK (a 1 MiB test
was attempted but the edit did not apply, so it remains genuinely untested),
feeding from a thread that pre-slices, or bypassing python-mpv's callback.

But note this is precisely the case for the project's Rust decision (SPEC 6):
a hot path that must sustain ~5 MB/s with hard per-frame deadlines is a poor
fit for a Python callback, and libmpv's stream_cb API is C -- so a Rust
implementation of this same design has none of this overhead.
"""

import argparse
import sys

import mpv

CHUNK = 1 << 16


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("stream", help="raw Annex-B HEVC elementary stream (.265)")
    p.add_argument("--fps", type=float, required=True,
                   help="frame rate of the stream (a raw stream carries no timestamps)")
    p.add_argument("--mode", default=None,
                   help="force a DRM mode, e.g. 3840x2160@30. Omitted = connector preferred.")
    p.add_argument("--demuxer-max-bytes", default="32MiB",
                   help="cap mpv's demuxer cache; the stream is infinite, so this must be bounded")
    p.add_argument("--dry-run", action="store_true", help="print the mpv options and exit")
    args = p.parse_args()

    # Read the loop into memory once. These are small (1.3 MB at 1080p, 14.8 MB
    # at 4K for a 3 s card) and it removes the filesystem from the hot path
    # entirely -- no re-open, no page-cache dependency, no I/O stall at the wrap.
    with open(args.stream, "rb") as fh:
        payload = fh.read()
    if not payload:
        print(f"error: {args.stream} is empty", file=sys.stderr)
        return 2

    opts = dict(
        vo="gpu",
        hwdec="drm",
        gpu_context="drm",
        gpu_api="opengl",
        gpu_hwdec_interop="drmprime-overlay",
        # Video goes on the primary plane and mpv's GL/OSD surface on the overlay:
        # this keeps the 4K video off the V3D render path entirely.
        drm_draw_plane="overlay",
        drm_drmprime_video_plane="primary",
        video_sync="display-resample",
        fullscreen=True,
        osc=False,
        input_default_bindings=False,
        terminal=False,
        # A raw elementary stream has no timestamps; mpv must generate them.
        correct_pts=False,
        container_fps_override=args.fps,
        # The stream never ends, so an unbounded cache would grow without limit.
        demuxer_max_bytes=args.demuxer_max_bytes,
    )
    if args.mode:
        opts["drm_mode"] = args.mode

    if args.dry_run:
        for k, v in opts.items():
            print(f"--{k.replace('_', '-')}={v}")
        print(f"(stream {args.stream}: {len(payload)} bytes, looped endlessly)")
        return 0

    player = mpv.MPV(**opts)

    @player.python_stream("loop")
    def _loop():
        # The generator never returns, so mpv never sees EOF -- which is the
        # entire mechanism. Yielding in chunks rather than one huge bytes object
        # keeps mpv's reads bounded and lets its cache limit do its job.
        while True:
            for off in range(0, len(payload), CHUNK):
                yield payload[off:off + CHUNK]

    player.play("python://loop")
    try:
        player.wait_for_playback()
    except KeyboardInterrupt:
        pass
    finally:
        player.terminate()
    return 0


if __name__ == "__main__":
    sys.exit(main())
