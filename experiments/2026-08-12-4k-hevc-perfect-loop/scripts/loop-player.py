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

STATUS 2026-08-14: DECODES BUT DOES NOT SCAN OUT. NOT YET USABLE.
-----------------------------------------------------------------
libmpv reports a fully healthy pipeline -- current-vo=gpu, hwdec-current=drm,
3840x2160, estimated-display-fps=29.9999, time-pos advancing, frame-drop-count
0 -- while the HDMI capture shows the Linux console, unchanged. The identical
option set passed to the mpv BINARY on the same machine displays perfectly, so
this is a libmpv-vs-CLI difference, not a wrong-options problem.

Untested hypotheses, cheapest first:
  1. DRM master acquisition differs when libmpv is embedded (the CLI logs
     "Can't open TTY for VT control" and works anyway; libmpv may fail to take
     master silently and render to an unscanned framebuffer).
  2. python-mpv creates the VO lazily or after some options are latched.
  3. Something in mpv's config/profile handling that the CLI applies and the
     library does not.

The proven-working equivalent, pending a fix, is the shell feed:
    while true; do cat loop.265; done | mpv <same options> -
That is measured seamless (zero held frames, 19 wraps at 4K30). This file is
kept because the in-process generator is the right SHAPE for M2 -- it removes
the per-loop process churn and the SIGPIPE hot-spin failure mode -- and only
the display half is unresolved.
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
