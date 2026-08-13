#!/usr/bin/env bash
# Gate: the Cam Link 4K must hold a SuperSpeed USB link, or 4K capture is worthless.
#
# The Cam Link advertises its available *input* modes according to the USB
# bandwidth it has. On a USB 2.0 link it stops offering 4K entirely and returns
# all-zero frames -- which decode to a flat green image, and which analyze.mjs
# reports as every frame failing to decode. That is indistinguishable from a
# catastrophic player failure, so it must be gated rather than eyeballed.
#
# Measured 2026-08-13: the same hub port renegotiated between SuperSpeed and
# USB 2.0 twice within one session, minutes apart. So this is run BEFORE and
# AFTER every capture -- a pre-flight check alone cannot see a mid-run drop.
#
# Exit 0 = SuperSpeed, capture is trustworthy. Non-zero = do not trust the run.
set -euo pipefail

DEVICE="Cam Link 4K"
MIN_SPEED=5000000000 # SuperSpeed, 5 Gb/s

usage() {
  echo "usage: $0 [--device NAME] [--quiet]" >&2
  exit 2
}

QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    --device) DEVICE="$2"; shift 2 ;;
    --quiet)  QUIET=1;     shift ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "check-capture-link: macOS only (uses ioreg); skipping" >&2
  exit 0
fi

speed=$(ioreg -rc IOUSBHostDevice | awk -v dev="+-o $DEVICE" '
  index($0, dev) { found = 1 }
  found && /"UsbLinkSpeed"/ { gsub(/[^0-9]/, "", $0); print; exit }
')

if [ -z "${speed:-}" ]; then
  echo "FAIL: '$DEVICE' not found on USB -- is it plugged in and powered?" >&2
  exit 1
fi

mbps=$((speed / 1000000))

if [ "$speed" -ge "$MIN_SPEED" ]; then
  [ "$QUIET" -eq 1 ] || echo "PASS: $DEVICE link = ${mbps} Mb/s (SuperSpeed) -- 4K capture viable"
  exit 0
fi

echo "FAIL: $DEVICE link = ${mbps} Mb/s -- below SuperSpeed." >&2
echo "      At this speed the card stops offering 4K modes and every captured" >&2
echo "      frame will be all-zero. Move it off any hub onto a USB 3 port." >&2
exit 1
