#!/usr/bin/env bash
# Measure whether an mpv configuration plays at realtime, and how long it takes
# to get there. Runs ON the Pi.
#
# This is a GATE, not a report. It must pass before any seam measurement is
# meaningful: a player running below realtime shows frames in slow motion or
# holds them, so its "wrap" is not the wrap a viewer would ever see.
#
# WHY mpv's OWN COUNTERS CANNOT REPLACE THIS. Measured 2026-08-13: mpv reported
# frame-drop-count=0, decoder-frame-drop-count=0 and vo-delayed-frame-count=0
# while genuinely presenting ~28.3 unique frames per second into a 30 Hz mode.
# It dropped nothing -- it HELD frames, roughly 1.7 times per second. The drop
# counters count frames mpv discards, not frames presented late or repeated, so
# they are structurally blind to this. Confirmed independently by counting loop
# wraps off an HDMI capture: 29 wraps in 92.11 s = 28.34 fps, not 30.
#
# WHY IT REPORTS STARTUP SEPARATELY. A single average over the whole run mixes a
# slow start into the steady state and reports neither. Measured on the same
# config: ratio 0.921 over a 10 s window, 0.967 over 30 s, 0.968 over 60 s --
# the average was still climbing at 30 s. So this samples a time series, finds
# where playback settles, and reports settle time and steady-state rate apart.
# Startup delay is a quality factor with a generous budget (dex players boot
# once, at install time); sustained rate error is not.
#
# Python is used for the IPC leg because Node is not installed on a Pi OS Lite
# image and python3 is; the rest of the harness stays Node-side on the Mac.
set -euo pipefail

VO="gpu"
HWDEC="drm"
ASSET=""
SECONDS_TO_SAMPLE=45
SETTLE_BUDGET=20 # seconds allowed before playback must be at steady rate
MIN_RATIO=0.98   # steady-state gate
TSV=""           # optional: write the raw time series here
SOCK="/tmp/mpv-rate.sock"

usage() {
  echo "usage: $0 --asset <file> [--vo VO] [--hwdec HW] [--sample SEC]" >&2
  echo "          [--settle-budget SEC] [--min-ratio R] [--tsv FILE] [--extra \"ARGS\"]" >&2
  exit 2
}

EXTRA=()
while [ $# -gt 0 ]; do
  case "$1" in
    --asset)          ASSET="$2";             shift 2 ;;
    --vo)             VO="$2";                shift 2 ;;
    --hwdec)          HWDEC="$2";             shift 2 ;;
    --sample)         SECONDS_TO_SAMPLE="$2"; shift 2 ;;
    --settle-budget)  SETTLE_BUDGET="$2";     shift 2 ;;
    --min-ratio)      MIN_RATIO="$2";         shift 2 ;;
    --tsv)            TSV="$2";               shift 2 ;;
    --extra)          read -r -a EXTRA <<<"$2"; shift 2 ;;
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
# Deliberately NOT sleeping before sampling: the startup ramp is part of what is
# being measured, so sampling starts the instant the IPC socket exists.
mpv --vo="$VO" --hwdec="$HWDEC" --fullscreen --no-osc \
  --no-input-default-bindings --no-terminal --loop-file=inf \
  --input-ipc-server="$SOCK" "${EXTRA[@]+"${EXTRA[@]}"}" "$ASSET" >/tmp/mpv-rate.log 2>&1 &
MPV_PID=$!

for _ in $(seq 1 60); do
  [ -S "$SOCK" ] && break
  sleep 0.25
done
[ -S "$SOCK" ] || { echo "FAIL: mpv did not create $SOCK; see /tmp/mpv-rate.log" >&2; exit 1; }

SAMPLE="$SECONDS_TO_SAMPLE" SOCKPATH="$SOCK" VO="$VO" HWDEC="$HWDEC" \
  BUDGET="$SETTLE_BUDGET" MINRATIO="$MIN_RATIO" TSVOUT="$TSV" python3 - <<'PY'
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

sample  = float(os.environ["SAMPLE"])
budget  = float(os.environ["BUDGET"])
minr    = float(os.environ["MINRATIO"])
tsv_out = os.environ.get("TSVOUT") or ""

# playback-time RESETS TO ZERO on every loop, so a naive (t1 - t0) over a window
# longer than the loop is garbage -- it under-reports and can go negative. Poll
# faster than the loop and accumulate, treating a backwards step as one wrap.
duration = q("duration") or 0.0
prev = q("playback-time")
if prev is None:
    print("FAIL: mpv did not report playback-time", file=sys.stderr)
    sys.exit(1)

STEP = 0.25
series = []          # (elapsed, cumulative_advanced)
advanced = 0.0
w0 = time.time()
while True:
    now = time.time()
    if now - w0 >= sample:
        break
    time.sleep(STEP)
    cur = q("playback-time")
    if cur is None:
        continue
    delta = cur - prev
    if delta < 0:
        delta += duration
    advanced += delta
    prev = cur
    series.append((time.time() - w0, advanced))

if len(series) < 8:
    print("FAIL: too few samples; increase --sample", file=sys.stderr)
    sys.exit(1)

if tsv_out:
    with open(tsv_out, "w") as fh:
        fh.write("elapsed_s\tadvanced_s\tinstant_ratio\n")
        for i, (t, a) in enumerate(series):
            if i == 0:
                inst = float("nan")
            else:
                pt, pa = series[i - 1]
                inst = (a - pa) / (t - pt) if t > pt else float("nan")
            fh.write(f"{t:.3f}\t{a:.3f}\t{inst:.4f}\n")

# Steady state = the last third of the run, which is past any startup ramp.
tail_start = series[len(series) * 2 // 3]
tail_end   = series[-1]
steady = (tail_end[1] - tail_start[1]) / (tail_end[0] - tail_start[0])

# Settle point = first sample after which every 1 s window stays within 2% of
# steady. Reported rather than assumed, so a slow start is visible instead of
# being averaged into the result.
WINDOW = max(4, int(1.0 / STEP))
settle = None
for i in range(len(series) - WINDOW):
    ok = True
    for j in range(i, len(series) - WINDOW):
        a0, a1 = series[j][1], series[j + WINDOW][1]
        t0, t1 = series[j][0], series[j + WINDOW][0]
        if t1 <= t0:
            continue
        if abs((a1 - a0) / (t1 - t0) - steady) > 0.02 * max(steady, 1e-9):
            ok = False
            break
    if ok:
        settle = series[i][0]
        break

overall = series[-1][1] / series[-1][0]
fps = steady * (q("container-fps") or 0)
settle_s = f"{settle:.1f}" if settle is not None else ">sample"

print(f"vo={os.environ['VO']:6s} hwdec={os.environ['HWDEC']:10s} "
      f"hwdec-current={q('hwdec-current')!s:10s}")
print(f"  steady_ratio={steady:.3f}  steady_fps={fps:.2f}  "
      f"overall_ratio={overall:.3f}")
print(f"  settle={settle_s}s (budget {budget:.0f}s)  "
      f"drops={q('frame-drop-count')}/{q('decoder-frame-drop-count')}/{q('vo-delayed-frame-count')}")
if tsv_out:
    print(f"  series -> {tsv_out}")

fail = []
if steady < minr:
    fail.append(f"steady_ratio {steady:.3f} < {minr}")
if settle is None or settle > budget:
    fail.append(f"settle {settle_s}s > budget {budget:.0f}s")
if fail:
    print("  FAIL: " + "; ".join(fail), file=sys.stderr)
    sys.exit(1)
print("  PASS")
PY
