#!/usr/bin/env bash
# Generate a lossless procedural test card whose motion loops exactly.
#
# The pattern is a pair of superimposed plane waves rotated by angle
# a = 2*PI*n/PERIOD. At n = PERIOD the angle is exactly 2*PI, so the frame is
# identical to n = 0 — the wrap is matched by construction, not by eye. That
# matters because otherwise the experiment would be measuring the asset rather
# than the player.
#
# Two spatial frequencies are superimposed on purpose: the low one gives smooth
# visible motion, the high one gives the encoder hard-to-compress detail so the
# decoder is honestly loaded at the target bitrate. A smooth gradient would
# compress to almost nothing and test the decoder at a bitrate no real artwork
# produces.
set -euo pipefail

usage() {
  echo "usage: $0 --width W --height H --fps F --frames N [--period P] --output out.mkv" >&2
  exit 2
}

WIDTH=""; HEIGHT=""; FPS=""; FRAMES=""; PERIOD=""; OUTPUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --width)  WIDTH="$2";  shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --fps)    FPS="$2";    shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --period) PERIOD="$2"; shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *) usage ;;
  esac
done
if [ -z "$WIDTH" ] || [ -z "$HEIGHT" ] || [ -z "$FPS" ] || [ -z "$FRAMES" ] || [ -z "$OUTPUT" ]; then
  usage
fi
PERIOD="${PERIOD:-$FRAMES}"

CX=$(( WIDTH / 2 ))
CY=$(( HEIGHT / 2 ))

# Rotated coordinate: u = (X-CX)*cos(a) + (Y-CY)*sin(a), a = 2*PI*mod(N,PERIOD)/PERIOD
#
# The mod() is load-bearing, not cosmetic. Without it, frame PERIOD evaluates
# cos(2*PI)/sin(2*PI), which are only *mathematically* equal to cos(0)/sin(0):
# in IEEE754, sin(2*PI) is -2.45e-16, and that was enough to flip 95 of 61440
# pixels by one luma level — so frame PERIOD was not bit-identical to frame 0.
# Wrapping the counter first makes both frames evaluate the identical
# expression, so the loop is exact by construction rather than by luck.
ANGLE="2*PI*mod(N\\,${PERIOD})/${PERIOD}"
ROT="((X-${CX})*cos(${ANGLE})+(Y-${CY})*sin(${ANGLE}))"
LUM="128+70*sin(0.06*${ROT})+50*sin(0.47*${ROT})"

ffmpeg -hide_banner -loglevel error \
  -f lavfi -i "nullsrc=s=${WIDTH}x${HEIGHT}:r=${FPS}" \
  -frames:v "$FRAMES" \
  -vf "geq=lum='${LUM}':cb=128:cr=128,format=yuv420p" \
  -c:v ffv1 -level 3 -an -y "$OUTPUT"

echo "test card: ${WIDTH}x${HEIGHT}@${FPS} ${FRAMES} frames (period ${PERIOD}) -> $OUTPUT" >&2
