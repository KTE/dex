#!/usr/bin/env bash
# Encode a barcoded lossless test card into the player-facing variants plus sidecars.
#
# GOP settings are not incidental: keyint=min-keyint=fps with scenecut=0 and
# open-gop=0 gives a closed GOP with an IDR every second and one at frame 0.
# An open GOP would make frame 0 depend on frames that no longer exist at the
# wrap, which is a way to manufacture the very seam we are trying to measure.
set -euo pipefail

usage() {
  echo "usage: $0 --input <barcoded.mkv> --outdir <dir> --name <base> --fps F [--bitrate 40M] [--h264]" >&2
  exit 2
}

INPUT=""; OUTDIR=""; NAME=""; FPS=""; BITRATE="40M"; WANT_H264=0
while [ $# -gt 0 ]; do
  case "$1" in
    --input)   INPUT="$2";   shift 2 ;;
    --outdir)  OUTDIR="$2";  shift 2 ;;
    --name)    NAME="$2";    shift 2 ;;
    --fps)     FPS="$2";     shift 2 ;;
    --bitrate) BITRATE="$2"; shift 2 ;;
    --h264)    WANT_H264=1;  shift ;;
    *) usage ;;
  esac
done
[ -n "$INPUT" ] && [ -n "$OUTDIR" ] && [ -n "$NAME" ] && [ -n "$FPS" ] || usage
mkdir -p "$OUTDIR"

# Comma is the csv writer's default separator; ffmpeg 8.1 rejects an explicit
# space separator ("Failed to parse option string 'p=0:s= '") and — worse —
# emits nothing rather than failing, so the dimensions would silently vanish.
DIMS="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=p=0 "$INPUT")"
IFS=',' read -r WIDTH HEIGHT <<< "$DIMS"
[ -n "$WIDTH" ] && [ -n "$HEIGHT" ] || { echo "could not read dimensions from $INPUT" >&2; exit 1; }

X265_PARAMS="keyint=${FPS}:min-keyint=${FPS}:scenecut=0:open-gop=0:repeat-headers=1"
ffmpeg -hide_banner -loglevel error -i "$INPUT" \
  -c:v libx265 -x265-params "$X265_PARAMS" \
  -b:v "$BITRATE" -pix_fmt yuv420p -tag:v hvc1 -an -y "$OUTDIR/$NAME.mp4"

if [ "$WANT_H264" -eq 1 ]; then
  # hello_video plays only raw H.264 elementary streams, so this is the
  # positive-control asset: the same test card the proven loop can play.
  ffmpeg -hide_banner -loglevel error -i "$INPUT" \
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
