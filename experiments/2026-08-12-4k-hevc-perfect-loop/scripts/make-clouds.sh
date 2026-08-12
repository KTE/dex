#!/usr/bin/env bash
# Generate a seamlessly-looping cloud texture, for compositing over a test card.
#
# WHY THIS EXISTS
# The dex test card is flat colours and static geometry, so x265 compresses it to
# roughly a tenth of what a real 4K artwork produces (3 Mbps at 1080p, 8.5 at 4K).
# A seam that only appears when the decoder is working hard would never show up.
# This adds texture so the decoder is loaded at a realistic bitrate.
#
# WHY NOT ffmpeg's noise FILTER
# Per-frame random noise reaches the bitrate but flickers, and flicker is exactly
# what masks a one-frame stutter for the human A/B — the thing the card's rotating
# hands are good at. Clouds drift instead.
#
# HOW IT LOOPS EXACTLY
# The texture is a sum of sinusoids whose spatial frequencies are integer cycles
# across the frame and whose temporal frequencies are integer cycles across the
# loop. That makes it periodic in x, y AND t by construction — so frame PERIOD is
# identical to frame 0. As with make-test-card.sh, the counter is wrapped with
# mod(N,PERIOD) BEFORE the trig, because sin(2*PI) is -2.45e-16 rather than 0 and
# that difference is enough to change pixels.
#
# An overlay that did not loop would manufacture a discontinuity at the wrap —
# i.e. it would fabricate the very defect the experiment is trying to detect.
set -euo pipefail

usage() {
  echo "usage: $0 --width W --height H --fps F --frames N [--period P] [--gain G] [--warp A] --output out.mkv" >&2
  echo "  --gain  contrast, default 40. Higher = harder edges, clips to marble." >&2
  echo "  --warp  domain-warp amplitude, default 0.07. Higher = more swirl, but" >&2
  echo "          the warp field's own lattice starts showing as periodic knots." >&2
  echo "  --coverage  sky/cloud threshold 0..1, default 0.55. This is what makes" >&2
  echo "          it cumulus rather than cirrus: everything below the threshold is" >&2
  echo "          clear sky, so the field breaks into separated masses instead of" >&2
  echo "          one continuous swirl. Higher = fewer, smaller clouds." >&2
  echo "  --puff  edge shaping exponent, default 0.65. Below 1 fattens the masses" >&2
  echo "          and softens their edges; above 1 makes them wispy." >&2
  exit 2
}

WIDTH=""; HEIGHT=""; FPS=""; FRAMES=""; PERIOD=""; OUTPUT=""; GAIN=40; WARP=0.07; COVERAGE=0.55; PUFF=0.65
while [ $# -gt 0 ]; do
  case "$1" in
    --width)  WIDTH="$2";  shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --fps)    FPS="$2";    shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --period) PERIOD="$2"; shift 2 ;;
    --gain)   GAIN="$2";   shift 2 ;;
    --warp)   WARP="$2";   shift 2 ;;
    --coverage) COVERAGE="$2"; shift 2 ;;
    --puff)   PUFF="$2";   shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$WIDTH" ] && [ -n "$HEIGHT" ] && [ -n "$FPS" ] && [ -n "$FRAMES" ] && [ -n "$OUTPUT" ] || usage
PERIOD="${PERIOD:-$FRAMES}"

# RENDER SMALL, UPSCALE. The highest octave is 23 cycles across the frame — a
# 167px feature at 4K — so there is no detail here that survives to pixel scale
# anyway. Rendering at quarter size and bicubic-upscaling is visually identical
# and ~16x faster, which is what makes domain warping affordable. Bitrate is
# unaffected because grain is added later, at encode time and at full size.
RW=$(( WIDTH / 4 )); [ "$RW" -lt 320 ] && RW="$WIDTH"
RH=$(( HEIGHT / 4 )); [ "$RH" -lt 180 ] && RH="$HEIGHT"

T="mod(N\\,${PERIOD})/${PERIOD}"
U="X/${RW}"
V="Y/${RH}"

# DOMAIN WARPING — the fix for lattice artefacts. Summed sinusoids evaluated on
# a plain grid always betray their lattice as plaid, however many octaves are
# added. Displacing the coordinates by another periodic field bends that lattice
# into something organic. Both warp fields are periodic in t, so the loop stays
# exact; spatial tiling is not required since the texture is never tiled.
WU="(${U}+${WARP}*sin(2*PI*(1*${U}+2*${V}+1*${T}+0.21)))"
WV="(${V}+${WARP}*sin(2*PI*(2*${U}-1*${V}-1*${T}+0.68)))"

# Octaves: fx, fy, ft, amplitude, phase. Roughly 1/f amplitudes, i.e. fBm-like.
#
# THIS LAYER IS FOR LOOK ONLY. An earlier version also carried high-frequency
# octaves (up to 251 cycles) to load the encoder, and it read as diagonal moiré
# rather than cloud: summed sinusoids at integer frequencies are regular
# interference, and octaves sharing a direction ratio reinforce into stripes.
# Measurement settled it — clouds plateau at ~11 Mbps however hard they are
# pushed, while a light grain added at ENCODE time reaches ~19. So grain carries
# the bitrate and this carries the look, which lets the fine octaves go entirely.
#
# Two rules keep it organic rather than patterned:
#   - directions must genuinely vary (mixed signs, non-proportional fx:fy),
#     otherwise the terms align into bands
#   - phases must differ, otherwise every term peaks at the origin together
#
# Every ft is +/-1 or +/-2: one or two drifts across the whole loop, so the
# texture moves like weather rather than static.
OCT=(
  "2 1 1 1.00 0.00"
  "3 -2 -1 0.70 0.37"
  "4 5 1 0.52 0.81"
  "7 -3 2 0.40 0.14"
  "6 -9 -1 0.32 0.62"
  "11 7 1 0.26 0.95"
  "9 -14 -2 0.21 0.28"
  "17 5 1 0.17 0.53"
)

SUM=""
for o in "${OCT[@]}"; do
  read -r FX FY FT AMP PH <<< "$o"
  TERM="${AMP}*sin(2*PI*(${FX}*${WU}+${FY}*${WV}+${FT}*${T}+${PH}))"
  if [ -z "$SUM" ]; then SUM="$TERM"; else SUM="${SUM}+${TERM}"; fi
done

# CUMULUS REMAP. Left as a linear ramp, summed noise reads as continuous swirl —
# cirrus or marble, not cumulus. Cumulus is defined by *separation*: discrete
# masses with clear sky between them. So normalise the field to 0..1, cut
# everything below COVERAGE to flat sky, and rescale what remains. The PUFF
# exponent then fattens the surviving masses so their edges bulge rather than
# taper.
TOTAL="$(awk 'BEGIN{s=0}{s+=$1}END{printf "%.4f", s}' <<< "$(printf '%s\n' "${OCT[@]}" | awk '{print $4}')")"
NORM="((${SUM})/${TOTAL}*0.5+0.5)"
CUT="clip((${NORM}-${COVERAGE})/(1-${COVERAGE})\\,0\\,1)"
LUM="clip(255*pow(${CUT}\\,${PUFF})*${GAIN}/40\\,0\\,255)"

ffmpeg -hide_banner -loglevel error \
  -f lavfi -i "nullsrc=s=${RW}x${RH}:r=${FPS}" \
  -frames:v "$FRAMES" \
  -vf "geq=lum='${LUM}':cb=128:cr=128,scale=${WIDTH}:${HEIGHT}:flags=bicubic,format=yuv420p" \
  -c:v ffv1 -level 3 -an -y "$OUTPUT"

echo "clouds: ${WIDTH}x${HEIGHT}@${FPS} ${FRAMES} frames (period ${PERIOD}, ${#OCT[@]} octaves, gain ${GAIN}, warp ${WARP}, coverage ${COVERAGE}, puff ${PUFF}, rendered ${RW}x${RH}) -> $OUTPUT" >&2
