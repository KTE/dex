#!/usr/bin/env bash
# Record the capture device losslessly, with timestamps, for offline analysis.
# Runs on the MAC (the capture host).
#
# Why record rather than decode live:
#
# 1. TIMESTAMPS. Live decoding yields a stream of indices with no time attached,
#    so it can say "a frame was held" but not "for how long, and when". A
#    recording carries each captured frame's presentation time, which turns the
#    barcode into a clock and lets display rate be measured directly instead of
#    inferred from the player's own self-report.
# 2. RE-DECODABILITY. It records the raw pixel band, not decoded indices, so the
#    same clip can be re-decoded with DIFFERENT barcode geometry later. That is
#    not hypothetical: on 2026-08-13 a concurrent session moved the barcode
#    mid-experiment and every live capture taken against the old asset was lost.
#    A recording would have survived it.
# 3. REPLAY. Analysis becomes a pure function of a file, so a revised analyzer
#    can be re-run over old evidence without re-occupying the bench.
#
# Only a horizontal BAND of the frame is kept, not the whole 4K picture: the
# barcode is all the timing analysis needs, and full 4K lossless would be ~450
# MB/s. The band is stored uncompressed-lossless (ffv1) so no encode artifact
# can ever reach the decoder.
set -euo pipefail

DEVICE="0"
SECONDS_TO_RECORD=20
OUT=""
BAND_Y_FRAC=0.80 # top of the kept band, as a fraction of frame height
BAND_H_FRAC=0.20 # height of the kept band
WIDTH=3840
HEIGHT=2160
FPS=30

usage() {
  echo "usage: $0 --out <file.mkv> [--seconds N] [--device N]" >&2
  echo "          [--size WxH] [--fps F] [--band-y FRAC] [--band-h FRAC]" >&2
  echo "  Records a horizontal band of the capture device, lossless, with timestamps." >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out)     OUT="$2";               shift 2 ;;
    --seconds) SECONDS_TO_RECORD="$2"; shift 2 ;;
    --device)  DEVICE="$2";            shift 2 ;;
    --fps)     FPS="$2";               shift 2 ;;
    --band-y)  BAND_Y_FRAC="$2";       shift 2 ;;
    --band-h)  BAND_H_FRAC="$2";       shift 2 ;;
    --size)    WIDTH="${2%x*}"; HEIGHT="${2#*x}"; shift 2 ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done
[ -n "$OUT" ] || usage

# The device index is NOT stable: plugging in a webcam renumbers everything.
# Resolve by name so a recording can never silently come from the wrong device.
NAME=$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 |
  awk -v d="\\[$DEVICE\\]" '/AVFoundation video devices/{v=1;next} /AVFoundation audio devices/{v=0} v && $0 ~ d {sub(/.*\] /,""); print; exit}')
echo "recording from avfoundation:${DEVICE} (${NAME:-unknown})" >&2

# -fps_mode passthrough is load-bearing: without it ffmpeg synthesises a
# constant-rate stream and duplicates frames to fill it, which fabricates
# exactly the held-frame artifact this recording exists to measure.
ffmpeg -hide_banner -loglevel error \
  -f avfoundation -pixel_format uyvy422 -video_size "${WIDTH}x${HEIGHT}" -framerate "$FPS" \
  -i "$DEVICE" \
  -fps_mode passthrough \
  -t "$SECONDS_TO_RECORD" \
  -vf "crop=iw:ih*${BAND_H_FRAC}:0:ih*${BAND_Y_FRAC},format=gray" \
  -c:v ffv1 -level 3 \
  -y "$OUT"

echo "wrote $OUT" >&2
ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,nb_read_frames -count_frames \
  -of default=nw=1 "$OUT" >&2
