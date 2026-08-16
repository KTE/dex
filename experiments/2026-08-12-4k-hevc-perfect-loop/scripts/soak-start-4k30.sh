#!/usr/bin/env bash
# Start the M5 24 h stability soak and its telemetry, then verify both survive
# a disconnection. Run AFTER soak-prep-4k30.sh and the reboot it asks for.
#
# Everything long-running lives in tmux ON THE PI. A backgrounded ssh command
# dies when the call returns, and every measurement taken afterwards describes a
# corpse -- including liveness checks, because `pgrep -f` also matches the ssh
# and timeout wrappers. This project has made that mistake more than once; see
# home-workspace docs/remote-long-running-processes.md.
#
# The player itself is NOT started here: the .deb enables dex-loop.service, so
# systemd starts it at boot. That is the deployment path under test. Starting it
# by hand would measure something else -- which is exactly how the accidental
# 90-minute run on 2026-08-15 ended up measuring a three-hour-old binary.
set -euo pipefail

PI="${PI:-dexpi@dexpi4.local}"
KEY="${KEY:-$HOME/.ssh/id_ed25519}"
ssh_pi() { ssh -o ConnectTimeout=10 -i "$KEY" "$PI" "$@"; }
say() { printf '\n=== %s ===\n' "$*"; }

say "1. the unit should already be playing (systemd started it at boot)"
ssh_pi 'systemctl is-active dex-loop || { echo "not active — starting"; sudo systemctl start dex-loop; sleep 15; }'
ssh_pi 'systemctl show dex-loop -p NRestarts --value | sed "s/^/  restarts so far: /"
        sudo journalctl -u dex-loop -n 3 --no-pager -o cat | sed "s/^/  /"'

say "2. confirm the mode actually took (a wrong mode is silent)"
ssh_pi 'modetest -M vc4 -c 2>/dev/null | grep -m1 "^  #0" | sed "s/^/  /"'

say "3. telemetry in tmux on the Pi"
ssh_pi 'cd ~/dex/experiments/2026-08-12-4k-hevc-perfect-loop
        tmux kill-session -t soak 2>/dev/null || true
        tmux new-session -d -s soak "./scripts/pi-telemetry.sh --interval 60 --proc dex-loop > ~/bench/soak-24h.tsv 2>~/bench/soak-24h.err"
        sleep 5
        tmux ls | sed "s/^/  /"'

say "4. prove it survives disconnection (the whole point of tmux)"
before=$(ssh_pi 'wc -l < ~/bench/soak-24h.tsv')
sleep 70
after=$(ssh_pi 'wc -l < ~/bench/soak-24h.tsv')
echo "  telemetry rows: $before -> $after (across two separate ssh sessions)"
[ "$after" -gt "$before" ] || { echo "  ERROR: telemetry did not advance — it did not survive." >&2; exit 1; }

say "5. the check command (tested here so it is known to work)"
cat <<'EOF'
  ssh dexpi@dexpi4.local '
    echo "--- player ---"; systemctl is-active dex-loop
    systemctl show dex-loop -p NRestarts --value | sed "s/^/restarts: /"
    sudo journalctl -u dex-loop --no-pager -o cat | grep heartbeat | tail -3
    echo "--- stall check: wraps must keep pace with uptime ---"
    sudo journalctl -u dex-loop --no-pager -o cat | grep heartbeat | tail -1 | \
      awk "{for(i=1;i<=NF;i++){if(\$i~/^wraps=/)w=substr(\$i,7);if(\$i~/^uptime=/)u=substr(\$i,8)}
            gsub(/s/,\"\",u); if(u>0) printf \"  %.2f s/wrap (asset is 3.00 s)\n\", u/w}"
    echo "--- thermal ---"; tail -2 ~/bench/soak-24h.tsv'
EOF
ssh_pi 'sudo journalctl -u dex-loop --no-pager -o cat | grep heartbeat | tail -1 | sed "s/^/  latest: /"'
echo
echo "Soak running. Pass bar: dex-loop/soak-24h/PASS-BAR.md"
