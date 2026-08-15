#!/usr/bin/env bash
# Long-run capture: chunk, analyze each chunk, keep only the summary.
#
# Storing the frame-index log rather than video is what makes a 24h soak
# continuously measured instead of sampled: 24h at 30fps is ~2.6M integers,
# which is nothing, while the equivalent video is unmanageable. Raw logs are
# kept only for chunks that did not pass, so a clean overnight run leaves a
# single summary file behind rather than thousands of uninteresting ones.
set -euo pipefail

usage() {
  echo "usage: $0 --source <path|avfoundation:N> --loop-length N --outdir <dir> [--chunk-frames 18000] [--chunks 0]" >&2
  echo "  --chunks 0 runs until interrupted" >&2
  exit 2
}

SOURCE=""; LOOP_LENGTH=""; OUTDIR=""; CHUNK_FRAMES=18000; CHUNKS=0
while [ $# -gt 0 ]; do
  case "$1" in
    --source)       SOURCE="$2";       shift 2 ;;
    --loop-length)  LOOP_LENGTH="$2";  shift 2 ;;
    --outdir)       OUTDIR="$2";       shift 2 ;;
    --chunk-frames) CHUNK_FRAMES="$2"; shift 2 ;;
    --chunks)       CHUNKS="$2";       shift 2 ;;
    *) usage ;;
  esac
done
if [ -z "$SOURCE" ] || [ -z "$LOOP_LENGTH" ] || [ -z "$OUTDIR" ]; then
  usage
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$OUTDIR"
SUMMARY="$OUTDIR/summary.tsv"
[ -f "$SUMMARY" ] || printf 'chunk\ttimestamp\tverdict\twrap_anom\twrap_n\tmid_anom\tmid_n\tp\n' > "$SUMMARY"

# One inline formatter rather than one node process per field.
READ_ROW='let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const r=JSON.parse(s);process.stdout.write([r.verdict,r.wrapAnomalies,r.wrapTransitions,r.midAnomalies,r.midTransitions,r.p].join("\t"));});'

FAILED=0
i=0
while [ "$CHUNKS" -eq 0 ] || [ "$i" -lt "$CHUNKS" ]; do
  LOG="$OUTDIR/chunk-$(printf '%05d' "$i").txt"
  node "$HERE/../bin/capture.mjs" --source "$SOURCE" \
    --out "$LOG" --frames "$CHUNK_FRAMES"

  # analyze exits non-zero on anything but PASS, and still prints its JSON, so
  # the exit status is the branch and the payload is read either way.
  if RESULT="$(node "$HERE/../bin/analyze.mjs" --log "$LOG" --loop-length "$LOOP_LENGTH" --json)"; then
    PASSED=1
  else
    PASSED=0
    FAILED=1
  fi

  ROW="$(printf '%s' "$RESULT" | node -e "$READ_ROW")"
  printf '%s\t%s\t%s\n' "$i" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$ROW" >> "$SUMMARY"

  if [ "$PASSED" -eq 1 ]; then
    rm -f "$LOG"
  else
    echo "chunk $i did not pass — log kept at $LOG" >&2
  fi

  i=$((i + 1))
done

exit "$FAILED"
