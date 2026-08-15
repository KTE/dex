#!/usr/bin/env bash
# Encode a barcoded lossless test card into the player-facing variants plus sidecars.
#
# GOP settings are not incidental: keyint=min-keyint=fps with scenecut=0 and
# open-gop=0 gives a closed GOP with an IDR every second and one at frame 0.
# An open GOP would make frame 0 depend on frames that no longer exist at the
# wrap, which is a way to manufacture the very seam we are trying to measure.
set -euo pipefail

usage() {
  echo "usage: $0 --input <barcoded.mkv> --outdir <dir> --name <base> --fps F [--bitrate 40M] [--grain N] [--h264]" >&2
  echo "  --grain adds light temporal noise AT ENCODE TIME so the decoder is loaded" >&2
  echo "     at a realistic bitrate. The test card alone compresses to ~10% of what" >&2
  echo "     real artwork produces (measured: 3.1 Mbps at 1080p against a 20M target)," >&2
  echo "     and a cloud overlay only reaches ~11. Grain 4 reaches 17.6, grain 8 reaches" >&2
  echo "     19.4. Keep it <= 12: the barcode is verified to survive that, not more." >&2
  exit 2
}

INPUT=""; OUTDIR=""; NAME=""; FPS=""; BITRATE="40M"; WANT_H264=0; GRAIN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --input)   INPUT="$2";   shift 2 ;;
    --outdir)  OUTDIR="$2";  shift 2 ;;
    --name)    NAME="$2";    shift 2 ;;
    --fps)     FPS="$2";     shift 2 ;;
    --bitrate) BITRATE="$2"; shift 2 ;;
    --grain)   GRAIN="$2";   shift 2 ;;
    --h264)    WANT_H264=1;  shift ;;
    *) usage ;;
  esac
done
if [ -z "$INPUT" ] || [ -z "$OUTDIR" ] || [ -z "$NAME" ] || [ -z "$FPS" ]; then
  usage
fi
mkdir -p "$OUTDIR"

# Comma is the csv writer's default separator; ffmpeg 8.1 rejects an explicit
# space separator ("Failed to parse option string 'p=0:s= '") and — worse —
# emits nothing rather than failing, so the dimensions would silently vanish.
DIMS="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=p=0 "$INPUT")"
IFS=',' read -r WIDTH HEIGHT <<< "$DIMS"
if [ -z "$WIDTH" ] || [ -z "$HEIGHT" ]; then
  echo "could not read dimensions from $INPUT" >&2; exit 1
fi

# Grain goes here rather than into the lossless intermediate: its whole purpose
# is to defeat compression, so baking it into an ffv1 source would bloat that
# file enormously for no benefit. It lands on top of the barcode, which is why
# build-bench-assets.sh re-decodes the finished encode to prove it survived.
GRAIN_VF=()
[ "$GRAIN" -gt 0 ] && GRAIN_VF=(-vf "noise=alls=${GRAIN}:allf=t+u")

X265_PARAMS="keyint=${FPS}:min-keyint=${FPS}:scenecut=0:open-gop=0:repeat-headers=1"
ffmpeg -hide_banner -loglevel error -i "$INPUT" "${GRAIN_VF[@]}" \
  -c:v libx265 -x265-params "$X265_PARAMS" \
  -b:v "$BITRATE" -pix_fmt yuv420p -tag:v hvc1 -an -y "$OUTDIR/$NAME.mp4"

if [ "$WANT_H264" -eq 1 ]; then
  # hello_video plays only raw H.264 elementary streams, so this is the
  # positive-control asset: the same test card the proven loop can play.
  ffmpeg -hide_banner -loglevel error -i "$INPUT" "${GRAIN_VF[@]}" \
    -c:v libx264 -x264-params "keyint=${FPS}:min-keyint=${FPS}:scenecut=0:open-gop=0" \
    -b:v "$BITRATE" -pix_fmt yuv420p -an -f h264 -y "$OUTDIR/$NAME.h264"
fi

cat > "$OUTDIR/$NAME.json" <<EOF
{
  "screens": {
    "HDMI-1": {
      "mode": [${WIDTH}, ${HEIGHT}, ${FPS}],
      "update_hz": ${FPS},
      "layers": [{
          "media": "${NAME}.mp4",
          "play": {"t": [0, 0], "rate": 1, "repeat": true},
          "buffer": 2
      }]
    }
  }
}
EOF

cat > "$OUTDIR/$NAME.html" <<EOF
<!-- for usage in a fullscreen browser like cog -->
<video autoplay muted loop src="${NAME}.mp4"></video>
EOF

echo "encoded ${WIDTH}x${HEIGHT}@${FPS} -> $OUTDIR/$NAME.{mp4,json,html}" >&2
