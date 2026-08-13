#!/usr/bin/env bash
# Build BOTH bench variants for one test card: clean and clouds.
#
# WHY BOTH
# The clean card encodes to ~8 Mbps and the clouds+grain one to ~39 Mbps, from
# identical source geometry. Running both at the bench is a discriminator: if
# clean passes and clouds fails, the cause is decoder load, not loop logic. One
# asset alone can only tell you THAT something failed.
#
# WHY THIS SCRIPT EXISTS AT ALL
# build-bench-assets.sh derives its output name from the input's dimensions and
# duration, so a clean build and a clouds build of the same card produce the SAME
# name — and the second silently overwrites the first. That happened on
# 2026-08-13. This wrapper gives the clouds variant its own suffix.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: build-variant.sh --card <lossless-card.mkv> [--clouds <dir-of-tif>] [--outdir out]
                        [--grain 7] [--bitrate auto] [--opacity 0.85]

  --card    a lossless master from packages/example-content/export/lossless/
  --clouds  directory of numbered TIFFs (f_0000.tif ...) from the AE cumulus
            comp, rendered at the SAME frame count and rate as the card.
            Omit to build only the clean variant.

Produces in --outdir:
  dex-test-card-<dur>s-<res><fps>.mp4          clean
  dex-test-card-<dur>s-<res><fps>-clouds.mp4   clouds + grain   (only with --clouds)
EOF
  exit 2
}

CARD=""; CLOUDS=""; OUTDIR="out"; GRAIN=7; BITRATE="auto"; OPACITY=0.85
while [ $# -gt 0 ]; do
  case "$1" in
    --card)    CARD="$2";    shift 2 ;;
    --clouds)  CLOUDS="$2";  shift 2 ;;
    --outdir)  OUTDIR="$2";  shift 2 ;;
    --grain)   GRAIN="$2";   shift 2 ;;
    --bitrate) BITRATE="$2"; shift 2 ;;
    --opacity) OPACITY="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$CARD" ] || usage
[ -f "$CARD" ] || { echo "no such card: $CARD" >&2; exit 1; }

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$OUTDIR/lossless"

read -r W H FPSR FRAMES <<< "$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,avg_frame_rate,nb_frames -of csv=p=0 "$CARD" | tr ',' ' ')"
if [ -z "$FRAMES" ] || [ "$FRAMES" = "N/A" ]; then
  FRAMES="$(ffprobe -v error -select_streams v:0 -count_frames \
    -show_entries stream=nb_read_frames -of csv=p=0 "$CARD")"
fi
FPS="$(awk -v r="$FPSR" 'BEGIN { split(r, a, "/"); printf "%g", a[1] / (a[2] ? a[2] : 1) }')"
DUR="$(awk -v f="$FRAMES" -v r="$FPS" 'BEGIN { printf "%g", f / r }')"
NAME="test-card-${DUR}s-${H}p${FPS}"

echo "==> ${W}x${H} @ ${FPS}fps, ${FRAMES} frames (${DUR}s)" >&2

# --- clean -------------------------------------------------------------------
bash "$HERE/build-bench-assets.sh" --input "$CARD" --outdir "$OUTDIR" --bitrate "$BITRATE"

[ -n "$CLOUDS" ] || { echo "==> clean only (no --clouds given)" >&2; exit 0; }

CLOUD_N="$(find "$CLOUDS" -name '*.tif' | wc -l | tr -d ' ')"
[ "$CLOUD_N" -eq "$FRAMES" ] || {
  echo "REFUSING: cloud sequence has $CLOUD_N frames, card has $FRAMES." >&2
  echo "  A cloud loop of a different length does not close where the card does," >&2
  echo "  which would plant a discontinuity at the wrap — the exact defect being measured." >&2
  exit 1
}

# --- matte: built from the ANIMATED card, never from the static artwork -------
# The static PNG's circle outline is narrower than the ring animated over it, so
# a matte keyed from the PNG lets texture onto the ring. See make-matte-from-video.
MATTE="$OUTDIR/lossless/matte-${NAME}.png"
node "$HERE/make-matte-from-video.mjs" --input "$CARD" --output "${MATTE%.png}.pgm" \
  --width "$W" --height "$H"
ffmpeg -hide_banner -loglevel error -i "${MATTE%.png}.pgm" -frames:v 1 -y "$MATTE"

# --- composite (lossless) ----------------------------------------------------
# Grain is NOT applied here: it belongs at encode time, because its whole purpose
# is to defeat compression and baking it into ffv1 would bloat this file hugely.
COMPED="$OUTDIR/lossless/${NAME}-clouds.mkv"
ffmpeg -hide_banner -loglevel error \
  -i "$CARD" \
  -framerate "$FPS" -i "$CLOUDS/f_%04d.tif" \
  -i "$MATTE" \
  -filter_complex "[0:v]format=yuv420p[card];[1:v]format=yuv420p[cl];[2:v]format=gray,format=yuv420p[m];[card][cl]blend=all_mode=overlay:all_opacity=${OPACITY}:shortest=1[tex];[card][tex][m]maskedmerge" \
  -c:v ffv1 -level 3 -an -y "$COMPED"

# --- clouds encode, into a DISTINCT name -------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
bash "$HERE/build-bench-assets.sh" --input "$COMPED" --outdir "$TMP" \
  --grain "$GRAIN" --bitrate "$BITRATE"
for ext in mp4 h264 json html; do
  [ -f "$TMP/dex-${NAME}.$ext" ] && mv "$TMP/dex-${NAME}.$ext" "$OUTDIR/dex-${NAME}-clouds.$ext"
done
# The sidecars name the media file; rewrite them to the -clouds name.
sed -i '' "s/dex-${NAME}\./dex-${NAME}-clouds./" \
  "$OUTDIR/dex-${NAME}-clouds.json" "$OUTDIR/dex-${NAME}-clouds.html" 2>/dev/null || true

echo "==> done: $OUTDIR/dex-${NAME}.mp4 (clean) and dex-${NAME}-clouds.mp4" >&2
