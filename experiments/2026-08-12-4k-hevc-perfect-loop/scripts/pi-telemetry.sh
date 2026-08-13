#!/usr/bin/env bash
# Sample Pi thermal and clock state during a probe run. Runs ON the Pi.
#
# Why this exists: thermal throttling presents as dropped frames, which is
# exactly the artifact this experiment counts. A run that throttled is not a
# failed run -- it is a VOID run, and the difference is only visible if the
# state was sampled while it happened. `vcgencmd get_throttled` reports both
# "now" bits and sticky "has occurred since boot" bits, so a run that throttled
# briefly is still detectable afterwards.
#
# Output is TSV on stdout, one row per sample, so it can be redirected next to
# the capture log and correlated by timestamp.
set -euo pipefail

INTERVAL=5
DURATION=0 # 0 = until killed

usage() {
  echo "usage: $0 [--interval SEC] [--duration SEC]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --interval) INTERVAL="$2"; shift 2 ;;
    --duration) DURATION="$2"; shift 2 ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

# Decode the throttled bitmask into something readable at a glance.
# Low bits are live state; the 0x1xxxx bits are sticky since boot.
decode_throttled() {
  local v=$1 out=""
  (((v & 0x1) != 0)) && out+="under-voltage-now,"
  (((v & 0x2) != 0)) && out+="arm-capped-now,"
  (((v & 0x4) != 0)) && out+="throttled-now,"
  (((v & 0x8) != 0)) && out+="soft-temp-limit-now,"
  (((v & 0x10000) != 0)) && out+="under-voltage-occurred,"
  (((v & 0x20000) != 0)) && out+="arm-capped-occurred,"
  (((v & 0x40000) != 0)) && out+="throttling-occurred,"
  (((v & 0x80000) != 0)) && out+="soft-temp-limit-occurred,"
  [ -z "$out" ] && out="clean,"
  echo "${out%,}"
}

printf 'iso_time\tuptime_s\ttemp_c\tarm_mhz\tcore_mhz\tvolts\tload1\tthrottled_hex\tthrottled_flags\tmpv_cpu\tmpv_rss_kb\n'

start=$(cut -d' ' -f1 /proc/uptime | cut -d. -f1)

while :; do
  now=$(cut -d' ' -f1 /proc/uptime | cut -d. -f1)
  [ "$DURATION" -gt 0 ] && [ $((now - start)) -ge "$DURATION" ] && break

  temp=$(vcgencmd measure_temp | sed 's/[^0-9.]//g')
  arm=$(($(vcgencmd measure_clock arm | cut -d= -f2) / 1000000))
  core=$(($(vcgencmd measure_clock core | cut -d= -f2) / 1000000))
  volts=$(vcgencmd measure_volts core | cut -d= -f2)
  load1=$(cut -d' ' -f1 /proc/loadavg)
  thr_hex=$(vcgencmd get_throttled | cut -d= -f2)
  thr_flags=$(decode_throttled "$((thr_hex))")

  # mpv may not be running yet, or may have exited; absent is not an error.
  read -r mpv_cpu mpv_rss <<<"$(ps -o %cpu=,rss= -C mpv 2>/dev/null | head -1)"
  mpv_cpu=${mpv_cpu:-NA}
  mpv_rss=${mpv_rss:-NA}

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(date -Is)" "$((now - start))" "$temp" "$arm" "$core" "$volts" \
    "$load1" "$thr_hex" "$thr_flags" "$mpv_cpu" "$mpv_rss"

  sleep "$INTERVAL"
done
