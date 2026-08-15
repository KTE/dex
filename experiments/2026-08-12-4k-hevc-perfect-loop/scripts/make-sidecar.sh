#!/usr/bin/env bash
# Generate (or --check) the F3 ingest sidecar dex-loop requires next to every
# raw Annex-B stream: <stream>.json, binding fps + sha256 to the asset bytes.
# See dex-loop/src/sidecar.rs for the contract this script targets and
# dex-loop/README.md for the manual recipe this replaces.
#
# Trust model: this script does NOT reimplement the sidecar grammar. Every
# sidecar it writes -- and every one passed to --check -- is round-tripped
# through the crate's REAL parser (Sidecar::from_json + verify_payload) via
# dex-loop/src/bin/sidecar-check.rs, a cargo bin that imports the very modules
# the player uses. A bash/jq reimplementation could silently drift from the
# parser it is meant to feed; this can't, because it never contains a second
# copy of the grammar. (It was a standalone rustc file until sidecar.rs and
# sha256.rs took on serde_json/sha2 -- SPEC 5c -- which rustc alone cannot
# resolve; it joined the crate rather than give up that property.)
#
# fps honesty: a raw Annex-B stream carries no container timestamps, so
# ffprobe's avg_frame_rate is a hardcoded 25/1 guess whenever it can't compute
# a real average, and r_frame_rate is only as good as the VUI timing_info the
# encoder happened to write into the SPS. Measured on this stream family: with
# VUI timing present, r_frame_rate is exact (24000/1001 -> 2997/125 etc);
# without it, ffprobe returns something like 1200000/1 (its internal
# timebase, not a frame rate). So a detected rate is trusted only inside a
# sane bound (see SANE_MIN/MAX below) -- outside it, or if ffprobe fails
# outright, this script refuses to guess and requires an explicit --fps.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$HERE/../dex-loop"

usage() {
  cat >&2 <<'EOF'
usage: make-sidecar.sh <stream.265> [--fps F] [--out FILE] [--force]
       make-sidecar.sh --check <stream.265> [--out FILE]

  <stream.265>   raw Annex-B HEVC elementary stream (the exact bytes dex-loop
                 will fs::read() at startup -- sha256 is computed over them
                 unmodified)
  --fps F        frame rate to bind, as dex-loop's sidecar grammar expects:
                 "30", "29.97", or "30000/1001". Overrides ffprobe detection.
                 If ffprobe also detected a rate and it disagrees beyond a
                 small tolerance, this REFUSES (exit 2) unless --force is
                 also given -- a wrong --fps plays the loop at the wrong
                 speed forever with every metric green, so a large
                 disagreement is treated as a likely typo, not honored
                 silently. Required whenever ffprobe cannot derive a
                 trustworthy rate from the stream itself (see fps honesty
                 note above).
  --out FILE     sidecar path. Default: <stream.265>.json (what dex-loop
                 looks for unconditionally: "<path>.json", no flag to change
                 it on the player side). In --check mode, the sidecar to
                 verify.
  --force        operator override for two separate refusals: (1) overwrite
                 an existing sidecar -- without it, an existing <out> file
                 refuses generation, so this script never silently clobbers
                 a prior ingest; (2) accept an explicit --fps that disagrees
                 with ffprobe's own detected rate beyond tolerance -- without
                 it, that combination refuses rather than guessing which
                 rate is right.
  --check        verify an existing sidecar against its stream (parses it
                 with the real parser and confirms sha256 over the exact
                 on-disk bytes) instead of generating one. Use before a soak.

exit codes: 0 = ok   1 = ffprobe/build/verification failure   2 = bad invocation
EOF
  exit 2
}

STREAM=""
FPS_ARG=""
OUT=""
FORCE=0
CHECK=0

while [ $# -gt 0 ]; do
  case "$1" in
    # `[ $# -ge 2 ]` before touching $2: under `set -u`, a flag as the LAST
    # token (an edited systemd unit, a line-continuation typo) would
    # otherwise dereference an unset $2 and abort with a raw "unbound
    # variable" -- loud, but the wrong exit code (1, "verification
    # failure") for what the exit-code contract at the top of this script
    # calls a bad invocation (2). Refuse through `usage` instead, same as
    # every other malformed-invocation case here.
    --fps)     [ $# -ge 2 ] || usage; FPS_ARG="$2"; shift 2 ;;
    --out)     [ $# -ge 2 ] || usage; OUT="$2";     shift 2 ;;
    --force)   FORCE=1;      shift ;;
    --check)   CHECK=1;      shift ;;
    -h|--help) usage ;;
    -*)        usage ;;
    *)
      [ -z "$STREAM" ] || usage
      STREAM="$1"
      shift
      ;;
  esac
done
[ -n "$STREAM" ] || usage
[ -f "$STREAM" ] || { echo "error: no such file: $STREAM" >&2; exit 1; }
[ -s "$STREAM" ] || { echo "error: $STREAM is empty" >&2; exit 1; }
[ -n "$OUT" ] || OUT="$STREAM.json"

# --- sha256 of the exact bytes: same call shape either sha256sum or shasum ---
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# --- build the real-parser round-trip checker once ---------------------------
# Built by cargo, not rustc: the checker moved into the crate (as
# dex-loop/src/bin/sidecar-check.rs) when sidecar.rs and sha256.rs gained
# serde_json/sha2 dependencies that plain rustc cannot resolve -- SPEC 5c. It
# still links the player's OWN parser and hash, which is the only reason this
# check means anything.
CHECKER_BIN=""
build_checker() {
  [ -n "$CHECKER_BIN" ] && return 0
  command -v cargo >/dev/null 2>&1 || {
    echo "error: cargo not found -- required to round-trip sidecars through" >&2
    echo "       the real dex-loop parser (dex-loop/src/bin/sidecar-check.rs);" >&2
    echo "       this script deliberately has no bash reimplementation of the" >&2
    echo "       sidecar grammar to fall back to." >&2
    exit 1
  }
  local log
  log="$(mktemp "${TMPDIR:-/tmp}/dex-sidecar-check.XXXXXX")"
  TMP_FILES+=("$log")
  if ! cargo build --quiet --release --manifest-path "$CRATE_DIR/Cargo.toml" \
       --bin sidecar-check >"$log" 2>&1; then
    echo "error: failed to build the sidecar round-trip checker:" >&2
    cat "$log" >&2
    exit 1
  fi
  CHECKER_BIN="$CRATE_DIR/target/release/sidecar-check"
  [ -x "$CHECKER_BIN" ] || {
    echo "error: cargo reported success but $CHECKER_BIN is not executable" >&2
    exit 1
  }
}

TMP_DIRS=()
TMP_FILES=()
cleanup() {
  # Must always return 0: under `set -e`, an EXIT trap whose own last
  # command fails clobbers the script's real exit code (e.g. a deliberate
  # `exit 2` silently becomes 1) -- exactly the silent-failure class this
  # project's whole design exists to avoid, so it must not happen here of
  # all places. (Bash also treats a zero-element array as "unset" for
  # `${arr[@]-default}`, which is why this loop is written with an
  # explicit `if`/`fi` rather than `[ ... ] && rm ...`: the latter's own
  # exit status, on the resulting empty-string iteration, is what leaked.)
  local d f
  for d in "${TMP_DIRS[@]-}"; do
    if [ -n "$d" ]; then rm -rf "$d"; fi
  done
  for f in "${TMP_FILES[@]-}"; do
    if [ -n "$f" ]; then rm -f "$f"; fi
  done
  return 0
}
trap cleanup EXIT

# --- fps detection from ffprobe, trusted only inside a sane bound -----------
# 1200000/1 is ffmpeg's fallback timebase when a raw HEVC SPS carries no VUI
# timing_info -- not a real frame rate. 1000 fps is a generous, deliberately
# round upper bound that lets any plausible camera/encoder rate through while
# still rejecting that fallback by three orders of magnitude.
SANE_MIN=1
SANE_MAX=1000

detect_fps() {
  local rate="$1"
  awk -v r="$rate" -v lo="$SANE_MIN" -v hi="$SANE_MAX" '
    BEGIN {
      n = split(r, a, "/")
      if (n != 2) { exit 1 }
      num = a[1] + 0; den = a[2] + 0
      if (num <= 0 || den <= 0) { exit 1 }
      val = num / den
      if (val < lo || val > hi) { exit 1 }
      x = num; y = den
      while (y != 0) { t = x % y; x = y; y = t }
      g = x
      rn = num / g; rd = den / g
      if (rd == 1) { printf "%d", rn } else { printf "%d/%d", rn, rd }
    }'
}

fps_to_decimal() {
  awk -v s="$1" 'BEGIN {
    if (s ~ /\//) { split(s, a, "/"); printf "%.6f", a[1] / a[2] }
    else          { printf "%.6f", s + 0 }
  }'
}

run_check() {
  build_checker
  [ -f "$OUT" ] || { echo "error: no sidecar at $OUT (nothing to check)" >&2; exit 1; }
  "$CHECKER_BIN" "$OUT" "$STREAM"
}

if [ "$CHECK" -eq 1 ]; then
  run_check
  exit $?
fi

# --- generate mode ------------------------------------------------------
if [ -e "$OUT" ] && [ "$FORCE" -ne 1 ]; then
  echo "error: $OUT already exists -- refusing to overwrite silently (pass --force)" >&2
  exit 2
fi

PROPS="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,r_frame_rate -of csv=p=0 "$STREAM" 2>/dev/null || true)"
WIDTH=""; HEIGHT=""; RATE=""
if [ -n "$PROPS" ]; then
  IFS=',' read -r WIDTH HEIGHT RATE <<< "$PROPS"
fi

DETECTED_FPS=""
if [ -n "$RATE" ]; then
  DETECTED_FPS="$(detect_fps "$RATE" || true)"
fi

if [ -n "$FPS_ARG" ]; then
  FINAL_FPS="$FPS_ARG"
  if [ -n "$DETECTED_FPS" ]; then
    given_dec="$(fps_to_decimal "$FPS_ARG")"
    detected_dec="$(fps_to_decimal "$DETECTED_FPS")"
    diff="$(awk -v a="$given_dec" -v b="$detected_dec" 'BEGIN { d = a - b; if (d < 0) d = -d; print d }')"
    if awk -v d="$diff" 'BEGIN { exit !(d > 0.02) }'; then
      # A wrong --fps binds the wrong rate into a hash-verified sidecar:
      # --check then passes FOREVER and the player runs slow/fast forever
      # with every metric green -- the one failure principle 5 says must be
      # made IMPOSSIBLE, not just warned about. A disagreement this large is
      # far more often an ingest typo (3 for 30, a copy-pasted wrong asset's
      # rate) than a deliberate override, so refuse unless the operator
      # confirms with --force -- the same flag that already means "I know
      # what I'm doing, proceed anyway" for the overwrite gate below.
      if [ "$FORCE" -ne 1 ]; then
        echo "error: --fps $FPS_ARG disagrees with ffprobe-detected rate $DETECTED_FPS" \
             "(from $STREAM's SPS VUI timing)." >&2
        echo "       Refusing rather than guessing which one is right -- a wrong fps" \
             "plays the loop at the wrong speed forever with every metric green." >&2
        echo "       If --fps $FPS_ARG is correct (ffprobe's guess is what's wrong)," \
             "pass --force to use it anyway. Otherwise drop --fps and trust the" \
             "detected rate." >&2
        exit 2
      fi
      echo "warning: --fps $FPS_ARG disagrees with ffprobe-detected rate $DETECTED_FPS" \
           "(from $STREAM's SPS VUI timing) -- proceeding because --force was given." >&2
    fi
  fi
elif [ -n "$DETECTED_FPS" ]; then
  FINAL_FPS="$DETECTED_FPS"
  echo "note: fps $FINAL_FPS derived from $STREAM's own SPS timing (ffprobe r_frame_rate)." \
       "Pass --fps explicitly if this asset's real rate is known some other way." >&2
else
  echo "error: could not derive a trustworthy fps for $STREAM." >&2
  echo "       A raw Annex-B stream has no container timestamps; ffprobe either" >&2
  echo "       failed to read it or returned a rate outside a plausible bound" >&2
  echo "       (i.e. no usable VUI timing_info in the SPS -- see the fps honesty" >&2
  echo "       note at the top of this script). Pass --fps <F> explicitly." >&2
  exit 2
fi

SHA256="$(sha256_of "$STREAM")"

WIDTH_JSON=""
HEIGHT_JSON=""
if [ -n "$WIDTH" ] && [ -n "$HEIGHT" ] && [ "$WIDTH" != "N/A" ] && [ "$HEIGHT" != "N/A" ]; then
  WIDTH_JSON=",\"width\":$WIDTH"
  HEIGHT_JSON=",\"height\":$HEIGHT"
else
  echo "note: width/height not available from ffprobe -- omitting (both are" \
       "optional/informational; dex-loop never reads them)." >&2
fi

TMP_OUT="$(mktemp "$OUT.XXXXXX")"
TMP_DIRS+=("$TMP_OUT")
printf '{"fps":"%s","sha256":"%s"%s%s}\n' \
  "$FINAL_FPS" "$SHA256" "$WIDTH_JSON" "$HEIGHT_JSON" > "$TMP_OUT"

# Verify what was just written through the real parser before it ever
# becomes the sidecar dex-loop will actually load -- generation that has
# only ever seen success is the exact bug class this script exists to close.
build_checker
if ! "$CHECKER_BIN" "$TMP_OUT" "$STREAM"; then
  echo "error: the sidecar this script just generated was REJECTED by the real" >&2
  echo "       parser -- this is a bug in make-sidecar.sh, not your input. Not" >&2
  echo "       writing $OUT." >&2
  exit 1
fi

# Existence was already gated above (refused unless --force); TMP_OUT is on
# the same filesystem as OUT (mktemp "$OUT.XXXXXX"), so this is atomic.
mv -f "$TMP_OUT" "$OUT"

echo "wrote $OUT" >&2
cat "$OUT"
