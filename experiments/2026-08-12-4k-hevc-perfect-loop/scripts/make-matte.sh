#!/usr/bin/env bash
# Build the background matte for the dex test card.
#
# WHY
# Texture (clouds, grain) must land ONLY on the grey background grid — never on
# the circles, bars, wedges, ramp or colour wheels, because those are the card's
# measurement elements and texturing them corrupts what they are for.
#
# WHY AN EXACT-VALUE KEY
# The background field is exactly RGB(211,211,211) and covers 58.7% of the frame.
# So no tolerance, no heuristics, and no hand-measured shapes are needed. Every
# other element differs, and anti-aliased edges land on intermediate values, so
# they fall outside the matte automatically — which leaves a clean one-pixel
# untextured halo around each element rather than a hard join.
#
# A tolerant key would be worse, not better: the greyscale ramp and the centre
# circle's gradient both pass through values near 211, so widening the key leaks
# into exactly the elements this is meant to protect.
#
# WHY THE MORPHOLOGICAL OPENING
# The centre circle's gradient crosses exactly 211 along a thin band, which the
# key picks up as a sliver INSIDE the circle. Grid cells are ~100px across at 4K
# and the sliver is a few px wide, so three erosions followed by three dilations
# removes the sliver and restores the cells. Note the grid LINES (value 38) stay
# excluded, so the texture sits between them and the grid stays crisp on top.
set -euo pipefail

usage() { echo "usage: $0 --input <card.png> --output <matte.png> [--no-open]" >&2; exit 2; }

INPUT=""; OUTPUT=""; DO_OPEN=1
while [ $# -gt 0 ]; do
  case "$1" in
    --input)   INPUT="$2";  shift 2 ;;
    --output)  OUTPUT="$2"; shift 2 ;;
    --no-open) DO_OPEN=0;   shift ;;
    *) usage ;;
  esac
done
if [ -z "$INPUT" ] || [ -z "$OUTPUT" ]; then
  usage
fi

KEY="255*eq(r(X\\,Y)\\,211)*eq(g(X\\,Y)\\,211)*eq(b(X\\,Y)\\,211)"
CHAIN="geq=r='${KEY}':g='${KEY}':b='${KEY}',format=gray"
[ "$DO_OPEN" -eq 1 ] && CHAIN="${CHAIN},erosion,erosion,erosion,dilation,dilation,dilation"

ffmpeg -hide_banner -loglevel error -i "$INPUT" -vf "$CHAIN" -frames:v 1 -y "$OUTPUT"

COV="$(ffmpeg -hide_banner -loglevel error -i "$OUTPUT" -f rawvideo -pix_fmt gray - 2>/dev/null \
  | node -e 'const c=[];process.stdin.on("data",d=>c.push(d)).on("end",()=>{const b=Buffer.concat(c);let w=0;for(const v of b)if(v>127)w++;process.stdout.write((100*w/b.length).toFixed(1));})')"
echo "matte: ${COV}% of frame is background -> $OUTPUT" >&2
