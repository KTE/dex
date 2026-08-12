#!/usr/bin/env bash
# Burn the binary frame-index barcode onto a lossless test card, losslessly.
# Geometry comes from lib/barcode.mjs so the burner and decoder cannot drift apart.
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
[ -n "$INPUT" ] && [ -n "$OUTPUT" ] || usage

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

HEIGHT="$(ffprobe -v error -select_streams v:0 -show_entries stream=height \
  -of csv=p=0 "$INPUT")"
FILTER="$(node "$HERE/../bin/barcode-filter.mjs" burn "$HEIGHT")"
STRIP="$(node "$HERE/../bin/barcode-filter.mjs" height "$HEIGHT")"

# ffv1 keeps the test card lossless: the barcode must survive to the encode
# stage unaltered, and any generational loss here would be indistinguishable
# from a decoder artifact later.
ffmpeg -hide_banner -loglevel error -i "$INPUT" \
  -vf "$FILTER" -c:v ffv1 -level 3 -an -y "$OUTPUT"

echo "burned barcode (${HEIGHT}px frame -> ${STRIP}px strip) -> $OUTPUT" >&2
