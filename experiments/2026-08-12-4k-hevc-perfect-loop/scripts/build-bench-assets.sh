#!/usr/bin/env bash
# Turn one lossless test-card export into the full set of bench assets.
#
# Input is a lossless master — normally the After Effects export of
# packages/example-content/test-cards/animation/test-cards.aep. Resolution,
# frame rate and duration are probed from the file rather than passed in, so
# the variant name always describes what the file actually is.
#
# Naming follows packages/example-content/export/, extended with frame rate
# because it distinguishes variants at 4K where it did not at 1080p:
#
#   out/lossless/test-card-2s-1080p30-barcoded.mkv   intermediate, barcoded
#   out/dex-test-card-2s-1080p30.mp4                 HEVC, the player asset
#   out/dex-test-card-2s-1080p30.h264                raw Annex-B, hello_video
#   out/dex-test-card-2s-1080p30.json                pivid timeline
#   out/dex-test-card-2s-1080p30.html                cog page
#
# The codec infix from the 2024 naming (-h265 / -h264) is dropped because here
# the extension already says it: HEVC is always .mp4, H.264 always .h264.
set -euo pipefail

usage() {
  echo "usage: $0 --input <lossless-master> [--outdir out] [--bitrate auto] [--target-fps F] [--grain N] [--h264]" >&2
  echo "  --target-fps decimates by an integer factor (source fps must be an exact multiple)." >&2
  echo "     Needed because the Cam Link records 4K at 30fps: pointed at 4K60 content it" >&2
  echo "     captures every other frame, and the analyzer sees indices stepping by 2." >&2
  exit 2
}

INPUT=""; OUTDIR="out"; BITRATE="auto"; WANT_H264=0; TARGET_FPS=""; GRAIN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --input)      INPUT="$2";      shift 2 ;;
    --outdir)     OUTDIR="$2";     shift 2 ;;
    --bitrate)    BITRATE="$2";    shift 2 ;;
    --target-fps) TARGET_FPS="$2"; shift 2 ;;
    --grain)      GRAIN="$2";      shift 2 ;;
    --h264)       WANT_H264=1;     shift ;;
    *) usage ;;
  esac
done
[ -n "$INPUT" ] || usage
[ -f "$INPUT" ] || { echo "no such file: $INPUT" >&2; exit 1; }

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PROPS="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,avg_frame_rate,nb_frames -of csv=p=0 "$INPUT")"
IFS=',' read -r WIDTH HEIGHT RATE FRAMES <<< "$PROPS"

# nb_frames is absent for some containers (notably ffv1 in mkv); counting is
# slower but always correct, and these clips are seconds long.
if [ -z "$FRAMES" ] || [ "$FRAMES" = "N/A" ]; then
  FRAMES="$(ffprobe -v error -select_streams v:0 -count_frames \
    -show_entries stream=nb_read_frames -of csv=p=0 "$INPUT")"
fi

FPS="$(awk -v r="$RATE" 'BEGIN { split(r, a, "/"); printf "%g", a[1] / (a[2] ? a[2] : 1) }')"
DUR="$(awk -v f="$FRAMES" -v r="$FPS" 'BEGIN { printf "%g", f / r }')"

[ -n "$WIDTH" ] && [ -n "$HEIGHT" ] && [ -n "$FRAMES" ] && [ "$FPS" != "0" ] \
  || { echo "could not probe $INPUT (got w=$WIDTH h=$HEIGHT frames=$FRAMES fps=$FPS)" >&2; exit 1; }

mkdir -p "$OUTDIR/lossless"

# Optional integer decimation. Legitimate for this content because it is
# synthetic graphics with no motion blur, so every Nth frame is an exact
# lower-rate sampling of the same motion — the rotations still complete over the
# loop, so the wrap stays matched.
if [ -n "$TARGET_FPS" ] && [ "$TARGET_FPS" != "$FPS" ]; then
  FACTOR="$(awk -v s="$FPS" -v t="$TARGET_FPS" 'BEGIN { printf "%g", s / t }')"
  case "$FACTOR" in
    *.*) echo "source ${FPS}fps is not an integer multiple of ${TARGET_FPS}fps (factor $FACTOR)" >&2; exit 1 ;;
  esac
  DECIMATED="$OUTDIR/lossless/decimated-${TARGET_FPS}fps.mkv"
  echo "==> decimating ${FPS}fps -> ${TARGET_FPS}fps (every ${FACTOR}th frame)" >&2
  ffmpeg -hide_banner -loglevel error -i "$INPUT" \
    -vf "select='not(mod(n\,${FACTOR}))',setpts=N/(${TARGET_FPS}*TB)" \
    -r "$TARGET_FPS" -c:v ffv1 -level 3 -an -y "$DECIMATED"
  INPUT="$DECIMATED"
  FRAMES="$(awk -v f="$FRAMES" -v x="$FACTOR" 'BEGIN { printf "%g", f / x }')"
  FPS="$TARGET_FPS"
  DUR="$(awk -v f="$FRAMES" -v r="$FPS" 'BEGIN { printf "%g", f / r }')"
fi

NAME="test-card-${DUR}s-${HEIGHT}p${FPS}"

# Bitrates chosen to load the decoder honestly while staying under the Pi 4's
# ~80 Mbps HEVC ceiling, which is the tighter of the two boards.
if [ "$BITRATE" = "auto" ]; then
  if [ "$HEIGHT" -ge 2000 ]; then BITRATE="40M"; else BITRATE="20M"; fi
fi

BARCODED="$OUTDIR/lossless/${NAME}-barcoded.mkv"

echo "==> ${WIDTH}x${HEIGHT} @ ${FPS}fps, ${FRAMES} frames (${DUR}s) -> ${NAME} @ ${BITRATE}" >&2

bash "$HERE/add-barcode.sh" --input "$INPUT" --output "$BARCODED"

EXTRA=()
[ "$WANT_H264" -eq 1 ] && EXTRA+=(--h264)
[ "$GRAIN" -gt 0 ] && EXTRA+=(--grain "$GRAIN")
bash "$HERE/encode-variants.sh" --input "$BARCODED" --outdir "$OUTDIR" \
  --name "dex-${NAME}" --fps "$FPS" --bitrate "$BITRATE" "${EXTRA[@]}"

# Verify the barcode survived the encode. A silent failure here would poison
# every measurement taken with this asset, so it is checked, not assumed.
TMPLOG="$(mktemp)"
trap 'rm -f "$TMPLOG"' EXIT
node "$HERE/../bin/capture.mjs" --source "$OUTDIR/dex-${NAME}.mp4" \
  --out "$TMPLOG" >/dev/null 2>&1

# shellcheck disable=SC2016  # the $ are JS template literals, not shell expansions;
# single quotes are exactly right here and double quotes would break the script.
node -e '
const fs = require("fs");
const v = fs.readFileSync(process.argv[1], "utf8").trim().split("\n")
  .map(x => x === "null" ? null : Number(x));
const bad = v.findIndex((n, i) => n !== i);
if (bad === -1) {
  console.error(`    barcode verified: ${v.length} frames decode exactly`);
} else {
  console.error(`    BARCODE VERIFY FAILED at frame ${bad}: got ${v[bad]}`);
  process.exit(1);
}
' "$TMPLOG"

echo "==> done: $OUTDIR/dex-${NAME}.*" >&2
