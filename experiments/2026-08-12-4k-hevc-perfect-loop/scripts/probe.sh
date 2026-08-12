#!/usr/bin/env bash
# Launch one named mpv loop configuration on the Pi.
#
# The three configurations are the three mechanisms from the dossier:
#   loop-file  - mpv's own looping; goes through EOF handling
#   ab-loop    - seeks before EOF is ever reached, so EOF handling is bypassed
#   keep-open  - holds the file open and seeks under external script control
#
# --dry-run prints the argv instead of running it. That argv IS the deliverable
# of this experiment: milestone 2's mpv.py play() shells out to exactly this.
set -euo pipefail

usage() {
  echo "usage: $0 --config <loop-file|ab-loop|keep-open> --asset <file> [--duration SEC] [--vo VO] [--hwdec HW] [--stats FILE] [--dry-run]" >&2
  exit 2
}

CONFIG=""; ASSET=""; DURATION=""; VO="drm"; HWDEC="drm"; STATS=""; DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --config)   CONFIG="$2";   shift 2 ;;
    --asset)    ASSET="$2";    shift 2 ;;
    --duration) DURATION="$2"; shift 2 ;;
    --vo)       VO="$2";       shift 2 ;;
    --hwdec)    HWDEC="$2";    shift 2 ;;
    --stats)    STATS="$2";    shift 2 ;;
    --dry-run)  DRY=1;         shift ;;
    *) usage ;;
  esac
done
[ -n "$CONFIG" ] && [ -n "$ASSET" ] || usage

# Kiosk flags shared by every configuration. A visible OSC or a stray keybinding
# would be indistinguishable from a player artifact in the capture.
ARGS=(mpv
  "--vo=${VO}"
  "--hwdec=${HWDEC}"
  --fullscreen
  --no-osc
  --no-input-default-bindings
  --no-terminal
  --msg-level=all=warn
)

[ -n "$STATS" ] && ARGS+=("--dump-stats=${STATS}")

case "$CONFIG" in
  loop-file)
    ARGS+=(--loop-file=inf)
    ;;
  ab-loop)
    [ -n "$DURATION" ] || { echo "--duration is required for the ab-loop config" >&2; exit 2; }
    # Stop one hundredth of a second short of the end so the seek happens before
    # EOF is ever reached — the entire point of this configuration.
    B="$(awk -v d="$DURATION" 'BEGIN { printf "%.3f", d - 0.01 }')"
    ARGS+=(--loop-file=inf "--ab-loop-a=0" "--ab-loop-b=${B}")
    ;;
  keep-open)
    ARGS+=(--keep-open=yes "--input-ipc-server=/tmp/mpv-probe.sock")
    ;;
  *)
    echo "unknown config: $CONFIG" >&2
    exit 2
    ;;
esac

ARGS+=("$ASSET")

if [ "$DRY" -eq 1 ]; then
  printf '%s\n' "${ARGS[@]}"
  exit 0
fi

exec "${ARGS[@]}"
