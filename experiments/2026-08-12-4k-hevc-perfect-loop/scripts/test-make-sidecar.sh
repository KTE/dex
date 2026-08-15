#!/usr/bin/env bash
# Tests make-sidecar.sh against a synthetic raw HEVC stream (not a bench
# asset -- this must run standalone on either the Mac or the Pi with nothing
# more than ffmpeg + rustc). Covers the two properties the task called out
# explicitly: a generator that has only ever seen success is untested, so
# this proves --check both passes on a good pairing AND fails on a corrupted
# one, plus the overwrite-refusal gate make-sidecar.sh is required to have.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/make-sidecar.sh"

PASS=0
FAIL=0

ok() {
  PASS=$((PASS + 1))
  echo "ok - $1"
}

not_ok() {
  FAIL=$((FAIL + 1))
  echo "NOT OK - $1" >&2
}

# expect_exit CODE DESC -- CMD...   runs CMD, asserts its exit code == CODE
expect_exit() {
  local want="$1" desc="$2"
  shift 2
  local got=0
  "$@" >"$WORK/last.out" 2>"$WORK/last.err" || got=$?
  if [ "$got" -eq "$want" ]; then
    ok "$desc (exit $got)"
  else
    not_ok "$desc (expected exit $want, got $got)"
    echo "  --- stdout ---"; sed 's/^/  /' "$WORK/last.out"
    echo "  --- stderr ---"; sed 's/^/  /' "$WORK/last.err"
  fi
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/test-make-sidecar.XXXXXX")"
# The trailing `return 0` matters: under `set -e`, an EXIT trap whose own
# last command fails clobbers the script's real exit code -- e.g. the final
# `[ "$FAIL" -eq 0 ]` below would report the wrong pass/fail verdict to the
# caller if cleanup ever failed on its own. See make-sidecar.sh's cleanup()
# for the concrete way this bit once already.
cleanup() { rm -rf "$WORK"; return 0; }
trap cleanup EXIT

STREAM="$WORK/loop.265"
SIDECAR="$STREAM.json"

# A tiny, real raw Annex-B HEVC elementary stream -- same shape as the bench
# assets (VUI timing present, since x265 writes it by default from -r), just
# small and synthetic so this test needs no external fixtures.
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=64x64:rate=30:duration=1 \
  -c:v libx265 -pix_fmt yuv420p -f hevc -y "$STREAM"
[ -s "$STREAM" ] || { echo "setup failed: could not synthesize $STREAM" >&2; exit 1; }

echo "=== generate ==="
expect_exit 0 "generate a sidecar for a fresh stream" "$SCRIPT" "$STREAM" --fps 30
if [ -f "$SIDECAR" ]; then ok "sidecar file was written"; else not_ok "sidecar file was written"; fi

echo "=== --check on a good pairing ==="
expect_exit 0 "--check passes on the matching stream+sidecar" "$SCRIPT" --check "$STREAM"

echo "=== overwrite refusal ==="
expect_exit 2 "generate refuses to overwrite without --force" "$SCRIPT" "$STREAM" --fps 30
expect_exit 0 "--force allows overwrite" "$SCRIPT" "$STREAM" --fps 30 --force

echo "=== corrupt a byte of the stream, --check must now fail ==="
# Flip one byte well inside the payload (not byte 0, to stay clear of any
# leading-NAL special-casing elsewhere in the codebase; this test is only
# about the sha256 binding, not NAL structure).
SIZE=$(wc -c < "$STREAM")
MID=$((SIZE / 2))
ORIG_BYTE="$(dd if="$STREAM" bs=1 skip="$MID" count=1 2>/dev/null | od -An -tu1 | tr -d ' ')"
FLIPPED=$(( (ORIG_BYTE + 1) % 256 ))
printf '%b' "\\x$(printf '%02x' "$FLIPPED")" | dd of="$STREAM" bs=1 seek="$MID" count=1 conv=notrunc 2>/dev/null

expect_exit 1 "--check FAILS once the stream is corrupted" "$SCRIPT" --check "$STREAM"
if grep -qi "sha256" "$WORK/last.err"; then
  ok "failure names sha256 as the reason"
else
  not_ok "failure names sha256 as the reason"
fi

# Re-synthesize: the byte flip above deliberately broke $STREAM's sha256, and
# every case below needs a stream ffprobe can still read cleanly.
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=64x64:rate=30:duration=1 \
  -c:v libx265 -pix_fmt yuv420p -f hevc -y "$STREAM"

echo "=== wildly wrong --fps is refused, not silently honored ==="
# gallery-ops review, 2026-08-15: a typo'd --fps (3 for 30) against a stream
# ffprobe CAN read a trustworthy rate from used to warn and proceed anyway --
# principle 5 says operator error must be made impossible, not detectable.
expect_exit 2 "--fps 3 (typo for 30) is refused without --force" \
  "$SCRIPT" "$STREAM" --fps 3 --out "$WORK/wrongfps.json"
if [ -f "$WORK/wrongfps.json" ]; then
  not_ok "refused --fps must not write a sidecar"
else
  ok "refused --fps did not write a sidecar"
fi
if grep -qi -- '--force' "$WORK/last.err"; then
  ok "refusal names --force as the way to override"
else
  not_ok "refusal names --force as the way to override"
fi
expect_exit 0 "--force overrides the wrong-fps refusal" \
  "$SCRIPT" "$STREAM" --fps 3 --out "$WORK/wrongfps.json" --force
if grep -q '"fps":"3"' "$WORK/wrongfps.json" 2>/dev/null; then
  ok "--force binds the operator-given fps, not the detected one"
else
  not_ok "--force binds the operator-given fps, not the detected one"
fi

echo "=== a trailing --fps with no value is a bad invocation, not a crash ==="
# rust review, 2026-08-15: under `set -u`, a bare trailing --fps used to
# dereference an unset $2 and abort with a raw "unbound variable" (exit 1)
# instead of refusing through the normal bad-invocation path (exit 2).
expect_exit 2 "trailing --fps with no value refuses via usage, not a crash" \
  "$SCRIPT" "$STREAM" --fps

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
