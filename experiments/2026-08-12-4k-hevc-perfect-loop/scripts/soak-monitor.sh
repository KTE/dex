#!/usr/bin/env bash
# Periodically sample a running loop and record whether it is still seamless.
# Runs on the MAC (the capture host); reads telemetry from the Pi over SSH.
#
# WHY A DEDICATED MONITOR
# -----------------------
# `soak.sh` answers "is the wrap anomaly rate above the noise floor" statistically.
# This answers a blunter question that turned out to be the decisive one:
# *is any frame being held?* On 2026-08-14 the seamless configuration produced
# dwell={1:N} -- every capture a distinct index, no repeats at all -- while the
# seeking configuration produced repeats concentrated at the loop's last frame.
# A held frame is therefore directly countable, and needs no statistics.
#
# Each sample also records the Pi's thermal state, because throttling presents
# as dropped frames and would otherwise be indistinguishable from a player fault.
#
# Output is one TSV row per sample so a multi-hour run can be read at a glance
# and plotted later. A row with held=0 is a clean sample.
set -euo pipefail

HOST="dexpi@dexpi4.local"
KEY="$HOME/.ssh/id_ed25519"
SOURCE="avfoundation:0"
FPS=30
FRAMES=1500
INTERVAL=900 # seconds between samples
LOOP_LENGTH=90
# The process to check for liveness. NOT always "mpv": dex-loop embeds libmpv,
# so there is no separate mpv process and checking for one reports a healthy
# player as dead.
PLAYER_PROC="dex-loop"
OUT="out/soak/monitor.tsv"
DURATION=0 # 0 = until interrupted
# Bound on the capture step. Observed 2026-08-15: ffmpeg can wedge mid
# avfoundation pixel-format renegotiation with the Cam Link, producing zero
# output and never exiting on its own. A real capture takes a few seconds;
# 20s is generous headroom, not a target.
CAPTURE_TIMEOUT=20

usage() {
  echo "usage: $0 [--out FILE] [--interval SEC] [--duration SEC] [--frames N]" >&2
  echo "          [--fps F] [--loop-length N] [--host USER@HOST]" >&2
  echo "          [--capture-timeout SEC]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out)             OUT="$2";             shift 2 ;;
    --interval)        INTERVAL="$2";        shift 2 ;;
    --duration)        DURATION="$2";        shift 2 ;;
    --frames)          FRAMES="$2";          shift 2 ;;
    --fps)             FPS="$2";             shift 2 ;;
    --loop-length)     LOOP_LENGTH="$2";     shift 2 ;;
    --player-proc)     PLAYER_PROC="$2";     shift 2 ;;
    --host)            HOST="$2";            shift 2 ;;
    --capture-timeout) CAPTURE_TIMEOUT="$2"; shift 2 ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

# Wraps ssh in a LOCAL wall-clock bound when `timeout` is available (GNU
# coreutils; present on this Mac via `brew install coreutils`, but not
# guaranteed on every machine this script might run on -- degrade to an
# unbounded call rather than fail outright, matching test-soak-monitor.sh's
# own `command -v timeout` pattern). This is belt-and-braces on top of the
# ssh-level ConnectTimeout/BatchMode/ServerAlive options and the remote-side
# `timeout 10` at the call site below; see the comment there for why all
# three layers are needed.
run_ssh_bounded() {
  if command -v timeout >/dev/null 2>&1; then
    timeout 20 ssh "$@"
  else
    ssh "$@"
  fi
}

mkdir -p "$(dirname "$OUT")"
# held_at is load-bearing, not decoration: a held frame AT THE WRAP is a player
# seam, while one at a random index is a capture-side duplicate, which SPEC 4.3
# documents as this instrument's noise floor. Logging only a COUNT (as the first
# version of this script did) makes the two indistinguishable after the fact --
# which is exactly what happened to the single held frame in the 2026-08-14 soak.
#
# One identity per invocation. Date alone is not enough: two runs can start in the
# same second, and then the merge this exists to expose would be invisible again.
RUN_ID="$(date +%Y%m%dT%H%M%S)-$$"

HEADER=$'run_id\tiso_time\telapsed_s\tframes\tnulls\twraps\theld\tmax_dwell\theld_at\tplayer\ttemp_c\tthrottled'

# Appending is only safe if the file already has THIS schema. The header was
# previously written only when the file was empty, so a schema change appended
# differently-shaped rows to an old file in silence -- which happened once, when
# held_at took the column count from 10 to 11.
if [ -s "$OUT" ]; then
  existing="$(head -1 "$OUT")"
  if [ "$existing" != "$HEADER" ]; then
    echo "soak-monitor: $OUT was written by a different schema; refusing to append." >&2
    echo "  expected: $HEADER" >&2
    echo "  found:    $existing" >&2
    echo "  move it aside or pass a different --out." >&2
    exit 3
  fi
else
  printf '%s\n' "$HEADER" >"$OUT"
fi

start=$(date +%s)
while :; do
  now=$(date +%s)
  [ "$DURATION" -gt 0 ] && [ $((now - start)) -ge "$DURATION" ] && break

  # The USB link must be SuperSpeed or the capture silently returns zeroed
  # frames; a soak that lost the link would otherwise log hours of "clean".
  if ! ./scripts/check-capture-link.sh >/dev/null 2>&1; then
    printf '%s\t%s\t%s\tLINK-FAIL\n' "$RUN_ID" "$(date -Iseconds)" "$((now - start))" >>"$OUT"
    sleep "$INTERVAL"
    continue
  fi

  tmp=$(mktemp)
  # Backgrounded and reaped with an explicit deadline, not called in the
  # foreground: ffmpeg has been observed to wedge mid avfoundation
  # pixel-format renegotiation, producing zero output and never exiting on
  # its own. A foreground call would then block this line -- and therefore
  # every row after it -- forever, which is a worse failure than the one this
  # monitor exists to catch: a soak that goes silent for its remaining hours
  # with no warning. On timeout, ffmpeg (node's child, not this shell's, so
  # `kill "$cap_pid"` alone would leave it running and holding the device) is
  # killed explicitly by parent pid. The empty/partial $tmp then flows into
  # the same node -e step below, which already reports an all-zero row for
  # zero captured frames -- an honest row, not a missing one.
  node bin/capture.mjs --source "$SOURCE" --out "$tmp" --frames "$FRAMES" --fps "$FPS" >/dev/null 2>&1 &
  cap_pid=$!
  waited=0
  while kill -0 "$cap_pid" 2>/dev/null; do
    if [ "$waited" -ge "$CAPTURE_TIMEOUT" ]; then
      echo "soak-monitor: capture exceeded ${CAPTURE_TIMEOUT}s, killing it (row will read zeroed, not LINK-FAIL)" >&2
      # Grab ffmpeg's pid via node's pid WHILE node is still alive to find it
      # by. Killing node first would orphan ffmpeg (reparenting it away from
      # $cap_pid), so a later "-P $cap_pid" lookup would then match nothing
      # and leak it -- observed: SIGTERM alone does not stop an ffmpeg wedged
      # in this state, only SIGKILL does, so both pids need a kill each.
      # pgrep -P can print MORE THAN ONE pid (capture.mjs spawns exactly one
      # child today, verified live, but a future refactor -- spawn via a
      # shell, add a second helper process -- would silently reopen this
      # leak if the kill below stayed a single unquoted-into-one-arg call).
      # Iterate instead of trusting it to always be a single pid.
      ffmpeg_pids=$(pgrep -P "$cap_pid" 2>/dev/null || true)
      kill -TERM "$cap_pid" 2>/dev/null || true
      if [ -n "$ffmpeg_pids" ]; then
        for p in $ffmpeg_pids; do kill -TERM "$p" 2>/dev/null || true; done
      fi
      sleep 1
      kill -KILL "$cap_pid" 2>/dev/null || true
      if [ -n "$ffmpeg_pids" ]; then
        for p in $ffmpeg_pids; do kill -KILL "$p" 2>/dev/null || true; done
      fi
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$cap_pid" 2>/dev/null || true

  # shellcheck disable=SC2016  # the $ are JS template literals, not shell expansions
  read -r frames nulls wraps held maxd heldat < <(node -e '
    import("node:fs").then(({readFileSync}) => {
      const v = readFileSync(process.argv[1], "utf8").trim().split("\n").filter(Boolean);
      const ok = v.filter(x => x !== "null").map(Number);
      if (!ok.length) { console.log(`${v.length} ${v.length} 0 0 0 -`); return; }
      const runs = []; let cur = ok[0], n = 1;
      for (const x of ok.slice(1)) { if (x === cur) n++; else { runs.push(n); cur = x; n = 1; } }
      runs.push(n);
      const L = Number(process.argv[2]);
      const wraps = ok.reduce((a, x, i) => a + (i && x < ok[i-1] ? 1 : 0), 0);
      // At ~27fps sampling a 30fps display every index should appear ONCE.
      // Two or more consecutive identical indices means the display held it.
      // Keep WHERE each hold happened. At the last frame of the loop it is a
      // player seam; at a random index it is a capture duplicate, i.e. noise.
      const idx = []; let c2 = ok[0], k = 1;
      for (const x of ok.slice(1)) { if (x === c2) k++; else { if (k >= 2) idx.push(c2); c2 = x; k = 1; } }
      if (k >= 2) idx.push(c2);
      const held = runs.filter(r => r >= 2).length;
      console.log(`${v.length} ${v.length - ok.length} ${wraps} ${held} ${Math.max(...runs)} ${idx.length ? idx.join(",") : "-"}`);
    });
  ' "$tmp" "$LOOP_LENGTH" || {
    # A node crash/OOM mid-soak must not silence the rest of the run the same
    # way an unbounded ffmpeg/ssh hang would -- log an honest zeroed row and
    # keep going, rather than letting `set -e` kill the whole monitor on a
    # `read` that got nothing from a failed process substitution.
    echo "soak-monitor: node parse step failed for this sample -- logging a zeroed row instead of going silent" >&2
    echo "0 0 0 0 0 -"
  })
  rm -f "$tmp"

  # `pgrep -c` PRINTS the count and EXITS NON-ZERO when it is 0, so a
  # `|| echo 0` fallback emits a second value and shifts every field after it.
  # Capture it plainly and default only if the variable is empty.
  #
  # BatchMode+ServerAlive bound a connection that establishes fine but then
  # goes quiet (network drop mid-read; ConnectTimeout alone only covers the
  # initial handshake). The remote `timeout 10` additionally bounds a
  # healthy-transport-but-wedged-remote-command case (e.g. a hung vcgencmd
  # firmware call) that no client-side ssh option can see, because from
  # ssh's point of view the connection itself is fine. `run_ssh_bounded`
  # wraps the whole call in a LOCAL timeout too, guarding this Mac's own
  # ssh client against a hang the remote-side bound cannot reach (e.g. a
  # wedge during key exchange). Without any of this, an unresponsive Pi
  # silences every row for the rest of a multi-day soak with no warning --
  # the same failure class the capture-step bound above exists to prevent.
  read -r player temp thr < <(run_ssh_bounded -i "$KEY" -o ConnectTimeout=10 \
    -o BatchMode=yes -o ServerAliveInterval=5 -o ServerAliveCountMax=2 "$HOST" \
    "timeout 10 sh -c 'c=\$(pgrep -c -x $PLAYER_PROC 2>/dev/null); printf \"%s %s %s\\n\" \"\${c:-0}\" \"\$(vcgencmd measure_temp | tr -dc 0-9.)\" \"\$(vcgencmd get_throttled | cut -d= -f2)\"'" \
    2>/dev/null || echo "? ? ?")

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$RUN_ID" "$(date -Iseconds)" "$((now - start))" "$frames" "$nulls" "$wraps" \
    "$held" "$maxd" "$heldat" "$player" "$temp" "$thr" >>"$OUT"

  sleep "$INTERVAL"
done
