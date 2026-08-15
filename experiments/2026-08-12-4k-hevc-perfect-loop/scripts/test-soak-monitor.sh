#!/usr/bin/env bash
# Self-test for soak-monitor.sh. A monitor that has only ever seen success is untested.
#
# The monitor's own sample loop needs the Cam Link (USB capture) and the Pi (over
# SSH) to produce a REAL sample -- but this test does not need either to be live.
# soak-monitor.sh already degrades on its own: no Cam Link -> an abbreviated
# LINK-FAIL row; a capture that decodes nothing -> a zeroed row; an unreachable
# Pi -> "?" telemetry fields. Every one of those still carries the run_id and
# obeys the header, which is all this test checks. Each invocation is bounded by
# a generous timeout anyway, so a genuinely wedged device (e.g. the Cam Link held
# by an unrelated live soak) fails this test loudly and fast instead of hanging
# the calling session forever.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

fails=0
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

# Each real sample is one capture + one SSH round trip; 45s is generous slack
# above the few seconds that takes when the hardware is present, and short
# enough that a wedged device fails this test in well under a minute rather
# than hanging it.
CAP=45
run_monitor() {
  if command -v timeout >/dev/null 2>&1; then
    timeout "$CAP" ./scripts/soak-monitor.sh "$@"
  else
    ./scripts/soak-monitor.sh "$@"
  fi
}

if ./scripts/check-capture-link.sh --quiet >/dev/null 2>&1; then
  echo "test-soak-monitor: Cam Link present -- samples below use live capture" >&2
else
  echo "test-soak-monitor: Cam Link absent/slow -- samples below degrade to LINK-FAIL rows (expected, not a failure)" >&2
fi

# 1. A fresh file gets a header whose first column is run_id.
run_monitor --out "$t/a.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
head -1 "$t/a.tsv" | grep -q $'^run_id\tiso_time' \
  || { echo "FAIL: header does not start with run_id"; fails=$((fails + 1)); }

# 2. Every data row carries a non-empty run_id, and one run uses exactly one value.
ids=$(awk -F'\t' 'NR>1 {print $1}' "$t/a.tsv" | sort -u | grep -c .)
[ "$ids" = "1" ] || { echo "FAIL: expected 1 run_id in one run, got $ids"; fails=$((fails + 1)); }

# 3. A SECOND run appended to the same file uses a DIFFERENT run_id, so the merge is
#    visible rather than silent. This is the defect that prompted the task.
sleep 1
run_monitor --out "$t/a.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
ids=$(awk -F'\t' 'NR>1 {print $1}' "$t/a.tsv" | sort -u | grep -c .)
[ "$ids" = "2" ] || { echo "FAIL: expected 2 run_ids after a restart, got $ids"; fails=$((fails + 1)); }

# 4. A file with a DIFFERENT schema is refused, not appended to. No hardware
#    involved: the schema check runs before the sample loop even starts.
printf 'iso_time\telapsed_s\tframes\n2026-01-01T00:00:00+00:00\t0\t1\n' > "$t/old.tsv"
run_monitor --out "$t/old.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
rc=$?
[ "$rc" = "3" ] || { echo "FAIL: expected exit 3 on schema mismatch, got $rc"; fails=$((fails + 1)); }
# BSD wc (macOS) right-pads `wc -l < file` with leading spaces ("       2", not
# "2"), so a bare string compare against "2" fails here even when the file is
# untouched. Strip whitespace so the check is honest on the platform this runs on.
lines=$(wc -l < "$t/old.tsv" | tr -d '[:space:]')
[ "$lines" = "2" ] || { echo "FAIL: monitor appended to a mismatched file (got $lines lines)"; fails=$((fails + 1)); }

[ "$fails" = "0" ] && echo "test-soak-monitor: PASS" || echo "test-soak-monitor: $fails FAILED"
exit "$fails"
