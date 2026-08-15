#!/usr/bin/env bash
# Burn the binary frame-index barcode onto a lossless test card, losslessly.
#
# The barcode lands centred and exactly on the test card's grid — 18 grid cells
# wide, one tall, in the lower third. Geometry comes from lib/barcode.mjs so the
# burner and decoder cannot drift apart, and it is expressed as fractions of the
# frame, so no resolution needs to be probed or passed.
set -euo pipefail

usage() { echo "usage: $0 --input <lossless-test-card> --output <out.mkv>" >&2; exit 2; }

INPUT=""; OUTPUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --input)  INPUT="$2";  shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *) usage ;;
  esac
done
if [ -z "$INPUT" ] || [ -z "$OUTPUT" ]; then
  usage
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FILTER="$(node "$HERE/../bin/barcode-filter.mjs" burn)"

# ffv1 keeps the test card lossless: the barcode must survive to the encode
# stage unaltered, and any generational loss here would be indistinguishable
# from a decoder artifact later.
ffmpeg -hide_banner -loglevel error -i "$INPUT" \
  -vf "$FILTER" -c:v ffv1 -level 3 -an -y "$OUTPUT"

DIMS="$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 "$OUTPUT")"
IFS=',' read -r W H <<< "$DIMS"
RECT="$(node "$HERE/../bin/barcode-filter.mjs" rect "$W" "$H")"
echo "burned barcode ${RECT} -> $OUTPUT" >&2
