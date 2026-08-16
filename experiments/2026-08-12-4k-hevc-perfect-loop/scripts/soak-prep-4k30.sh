#!/usr/bin/env bash
# Prepare the Pi for the M5 24 h stability soak at 4K30, and start it.
#
# WHY 4K30 AND NOT THE 1440p60 SHIPPING MODE (Max, 2026-08-16). The artwork's
# 1440p60 build was already exercised for 2 h in the sealed-box thermal test:
# 199 wraps, zero dropped frames, zero VO delays, peak 78.4 C. That answered the
# question that run was asked. What is still unmeasured is LONG-RUN STABILITY --
# memory growth, accumulated drops, crashes, restarts -- and for that the right
# asset is the HEAVIER one, because a day at 4K30 subsumes a day at 1440p60:
#
#     4K30    3840x2160x30 = 249 Mpix/s
#     1440p60 2560x1440x60 = 221 Mpix/s
#
# DELIBERATELY NO CAPTURE. Earlier soaks used the Cam Link to count held frames
# from a burned-in barcode. That instrument answers the SEAM question (M1,
# closed) and needs 2x oversampling to work at all -- 1080p60 capture against
# 30 fps content. At 4K the Cam Link captures ~27 fps against 30 fps content,
# i.e. UNDER-sampled, so it cannot resolve a held frame even in principle. It
# would look like an independent witness and testify to nothing.
#
# What replaces it: mpv's own frame-drop/vo-delayed counters (cross-validated
# against capture during M1, which is what makes trusting them defensible), plus
# a stall check needing no capture at all -- WRAPS DIVIDED BY UPTIME. A wedged
# player keeps incrementing uptime while wraps freeze, so the ratio catches from
# the log alone what capture would have caught.
set -euo pipefail

PI="${PI:-dexpi@dexpi4.local}"
KEY="${KEY:-$HOME/.ssh/id_ed25519}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASSET_SRC="${ASSET_SRC:-$HOME/.claude/jobs/1a5217b5/tmp/nts-4k30-25M.265}"
MODE="3840x2160@30"

ssh_pi() { ssh -o ConnectTimeout=10 -i "$KEY" "$PI" "$@"; }
say() { printf '\n=== %s ===\n' "$*"; }

say "0. reachable?"
ssh_pi 'uptime -p' || { echo "Pi unreachable — power it on first." >&2; exit 1; }

say "1. which display is connected?"
# The Cam Link and the Dell need OPPOSITE cmdline.txt settings, and getting it
# wrong is silent: the Pi transmits a mode the sink cannot show and it reads as a
# player fault. Identify the sink rather than assuming.
SINK=$(ssh_pi "edid-decode /sys/class/drm/card1-HDMI-A-1/edid 2>/dev/null | grep -i 'Display Product Name' | head -1 | sed 's/.*: //' | tr -d \"'\"" || echo "unknown")
echo "  sink: $SINK"
case "$SINK" in
  *"Cam Link"*) ;;
  *) echo "  ERROR: expected the Cam Link (the only 4K-capable sink here)." >&2
     echo "  The Dell U2719DC is 2560x1440 and its EDID never mentions 2160." >&2
     exit 2 ;;
esac

say "2. restore the 4K30 force (the Cam Link needs it; the Dell must not have it)"
# vc4 refuses to build 3840x2160 from the Cam Link's EDID even though the EDID
# offers it as its PREFERRED detailed timing. Verified both directions
# 2026-08-15. The explanation lives in a comment block in config.txt.
ssh_pi "grep -q 'video=HDMI-A-1:$MODE' /boot/firmware/cmdline.txt \
        || sudo sed -i \"s|\\\$| video=HDMI-A-1:$MODE|\" /boot/firmware/cmdline.txt"
ssh_pi "tr ' ' '\n' < /boot/firmware/cmdline.txt | grep video= | sed 's/^/  /'"

say "3. point the unit at 4K30 (the drop-in from the 1440p plan forces 2560x1440)"
ssh_pi "sudo mkdir -p /etc/systemd/system/dex-loop.service.d
        printf '[Service]\nExecStart=\nExecStart=/usr/bin/dex-loop /opt/dex/loop.265 --mode $MODE\n' \
          | sudo tee /etc/systemd/system/dex-loop.service.d/override.conf >/dev/null
        sudo systemctl daemon-reload"
ssh_pi "cat /etc/systemd/system/dex-loop.service.d/override.conf | sed 's/^/  /'"

say "4. deploy the 4K30 asset + sidecar"
[ -f "$ASSET_SRC" ] || { echo "asset not found: $ASSET_SRC" >&2; exit 1; }
ssh_pi 'sudo systemctl stop dex-loop 2>/dev/null || true'
scp -q -i "$KEY" "$ASSET_SRC" "$PI:/tmp/loop.265"
ssh_pi 'sudo install -m644 /tmp/loop.265 /opt/dex/loop.265 && rm -f /tmp/loop.265'
ssh_pi "cd ~/dex/experiments/2026-08-12-4k-hevc-perfect-loop \
        && sudo rm -f /opt/dex/loop.265.json \
        && ./scripts/make-sidecar.sh --out /tmp/loop.265.json /opt/dex/loop.265 \
        && sudo install -m644 /tmp/loop.265.json /opt/dex/loop.265.json \
        && ./scripts/make-sidecar.sh --check /opt/dex/loop.265"

say "5. record run identity BEFORE starting (a soak whose build is unknown measured nothing)"
ssh_pi "dpkg-query -W -f='  package: \${Version}\n' dex-loop; dex-loop --version | sed 's/^/  binary:  /'
        sha256sum /opt/dex/loop.265 | sed 's/^/  asset:   /'
        git -C ~/dex log --oneline -1 | sed 's/^/  repo:    /'"

echo
echo "cmdline.txt changed -> a REBOOT is required for the mode to apply."
echo "Run:  ssh $PI 'sudo reboot'   then:  $HERE/soak-start-4k30.sh"
