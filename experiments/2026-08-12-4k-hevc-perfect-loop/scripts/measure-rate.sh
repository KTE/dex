#!/usr/bin/env bash
# Measure whether an mpv configuration actually plays at realtime. Runs ON the Pi.
#
# This is a GATE, not a report, and it must pass before any seam measurement is
# meaningful: a player running at 0.15x shows every frame in slow motion, so its
# "wrap" is not the wrap a viewer would ever see. Measuring the seam first and
# the rate afterwards would produce a confident verdict about nothing.
#
# The subtlety that makes this necessary: mpv reported frame-drop-count=0,
# decoder-frame-drop-count=0 and vo-delayed-frame-count=0 while running at 0.147x.
# It was not dropping frames -- it was presenting all of them, far too slowly.
# So the drop counters cannot be used to detect this condition. Only the ratio of
# playback-time to wall-clock time can.
#
# Python is used for the IPC leg because Node is not installed on a Pi OS Lite
# image and python3 is; the rest of the harness stays Node-side on the Mac.
set -euo pipefail

VO="gpu"
HWDEC="drm"
ASSET=""
SECONDS_TO_SAMPLE=10
SOCK="/tmp/mpv-rate.sock"

usage() {
  echo "usage: $0 --asset <file> [--vo VO] [--hwdec HW] [--sample SEC]" >&2
  exit 2
}

EXTRA=()
while [ $# -gt 0 ]; do
  case "$1" in
    --asset)  ASSET="$2";            shift 2 ;;
    --vo)     VO="$2";               shift 2 ;;
    --hwdec)  HWDEC="$2";            shift 2 ;;
    --sample) SECONDS_TO_SAMPLE="$2"; shift 2 ;;
    --extra)  read -r -a EXTRA <<<"$2"; shift 2 ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done
[ -n "$ASSET" ] || usage

cleanup() {
  [ -n "${MPV_PID:-}" ] && kill "$MPV_PID" 2>/dev/null
  rm -f "$SOCK"
}
trap cleanup EXIT

rm -f "$SOCK"
mpv --vo="$VO" --hwdec="$HWDEC" --fullscreen --no-osc \
  --no-input-default-bindings --no-terminal --loop-file=inf \
  --input-ipc-server="$SOCK" "${EXTRA[@]+"${EXTRA[@]}"}" "$ASSET" >/tmp/mpv-rate.log 2>&1 &
MPV_PID=$!

# Wait for the socket rather than sleeping a guessed interval.
for _ in $(seq 1 40); do
  [ -S "$SOCK" ] && break
  sleep 0.25
done
[ -S "$SOCK" ] || { echo "FAIL: mpv did not create $SOCK; see /tmp/mpv-rate.log" >&2; exit 1; }
sleep 2 # let playback settle past startup

SAMPLE="$SECONDS_TO_SAMPLE" SOCKPATH="$SOCK" VO="$VO" HWDEC="$HWDEC" python3 - <<'PY'
import socket, json, time, os, sys

sock = socket.socket(socket.AF_UNIX)
sock.connect(os.environ["SOCKPATH"])
buf = b""

def q(prop):
    global buf
    sock.sendall(json.dumps({"command": ["get_property", prop]}).encode() + b"\n")
    while True:
        buf += sock.recv(65536)
        lines = buf.split(b"\n")
        buf = lines.pop()
        for line in lines:
            if not line.strip():
                continue
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if "error" in m:
                return m.get("data")

sample = float(os.environ["SAMPLE"])

# playback-time RESETS TO ZERO on every loop, and this asset is a 3 s loop.
# A naive (t1 - t0) / wall over any window longer than the loop is therefore
# garbage -- it under-reports badly and can even go negative. That mistake
# produced a confident "0.147x, playback is in slow motion" reading from a
# player that was in fact running at realtime.
# So: poll faster than the loop length and accumulate, treating any backwards
# step as a wrap worth one loop duration.
duration = q("duration") or 0.0
prev = q("playback-time")
if prev is None:
    print("FAIL: mpv did not report playback-time", file=sys.stderr)
    sys.exit(1)

advanced = 0.0
w0 = time.time()
while time.time() - w0 < sample:
    time.sleep(0.2)
    cur = q("playback-time")
    if cur is None:
        continue
    delta = cur - prev
    if delta < 0:  # wrapped
        delta += duration
    advanced += delta
    prev = cur
w1 = time.time()

ratio = advanced / (w1 - w0)
fps = ratio * (q("container-fps") or 0)
print(f"vo={os.environ['VO']:6s} hwdec={os.environ['HWDEC']:10s} "
      f"hwdec-current={q('hwdec-current')!s:10s} "
      f"ratio={ratio:.3f} effective_fps={fps:.1f} "
      f"drops={q('frame-drop-count')}/{q('decoder-frame-drop-count')}/{q('vo-delayed-frame-count')}")

# 0.98 rather than 1.0: sampling jitter over a short window is not a failure.
sys.exit(0 if ratio >= 0.98 else 1)
PY
