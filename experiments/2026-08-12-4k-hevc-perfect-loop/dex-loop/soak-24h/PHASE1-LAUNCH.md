# Phase 1 — run this after the physical swap + power-on

Everything up to here (package install, asset+sidecar deploy, mode override,
cmdline.txt) is already done and verified. This is what's left, in order.
Paste block by block, or run as a script.

## 0. Physical step (not remote)

1. Power off if not already (it was left off deliberately after the cmdline.txt
   edit — that edit only takes effect on the next boot).
2. Cam Link 4K out, Dell U2719DC in (HDMI-A-1).
3. Dell OSD: disable auto-sleep / DPMS timeout.
4. Confirm the case fan is running (box is open, not sealed, for this run).
5. Power on.

## 1. Within ~5 minutes of boot

```bash
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local

mkdir -p ~/soak-24h
systemctl show dex-loop -p NRestarts,MainPID,ExecMainStartTimestamp | tee ~/soak-24h/start-state.txt
vcgencmd get_throttled | tee -a ~/soak-24h/start-state.txt        # expect 0x0

cat /sys/class/drm/card*-HDMI-A-1/status                           # connected
cat /sys/class/drm/card*-HDMI-A-1/modes | head -3                  # expect 2560x1440 first entry
# If it's NOT 2560x1440 -> VOID, fix (F6-style cmdline force for the Dell), restart clock.

journalctl -u dex-loop -b --no-pager | head -40
# Expect: dex-wait-hdmi succeeded, sidecar accepted (fps=60000/1001), playing,
# no "refused" / exit-2 lines.
```

## 2. Telemetry — start AFTER boot (tmux does not survive reboot)

```bash
tmux new-session -d -s soak24
tmux send-keys -t soak24 'cd ~/dex/experiments/2026-08-12-4k-hevc-perfect-loop && ./scripts/pi-telemetry.sh --interval 30 --proc dex-loop > ~/soak-24h/telemetry.tsv 2> ~/soak-24h/telemetry.err' Enter
sleep 5
tmux capture-pane -t soak24 -p | tail -5    # confirm rows flowing
```

## 3. Eyeball check #1

Look at the Dell: landscape orientation, smooth motion, no hang. Log the
observation (timestamp + note) in `~/soak-24h/eyeball.log`. Then disconnect —
the run is fully self-contained on the Pi from here (systemd + persistent
journal + tmux telemetry).

## 4. One-command progress check (from the Mac, any time)

```bash
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local '
systemctl show dex-loop -p NRestarts,ActiveState,ExecMainStartTimestamp
echo "---"
journalctl -u dex-loop --no-pager | grep heartbeat | tail -3
echo "---"
tmux capture-pane -t soak24 -p | tail -3
'
```

Restarts should stay 0, the last heartbeat should be <700s old, wraps should
be climbing, and the telemetry pane should show recent rows.

## 5. Teardown at ≥24h00m (see pass-bar doc for full scoring)

```bash
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local
systemctl show dex-loop -p NRestarts,MainPID,ExecMainStartTimestamp | tee ~/soak-24h/end-state.txt
vcgencmd get_throttled | tee -a ~/soak-24h/end-state.txt
journalctl --list-boots | tee -a ~/soak-24h/end-state.txt
journalctl -u dex-loop -b --no-pager > ~/soak-24h/journal.txt
tmux send-keys -t soak24 C-c   # stop telemetry; leave the unit running

# Mac:
scp -i ~/.ssh/id_ed25519 'dexpi@dexpi4.local:~/soak-24h/*' out/soak-24h/
grep heartbeat out/soak-24h/journal.txt
```

Score against `SOAK-24H-PASS-BAR.md`'s table. Record the verdict + the
vo-delayed 24h baseline in the story.

## Restoring the bench (Cam Link) setup afterward

```bash
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local '
sudo cp /boot/firmware/cmdline.txt.camlink-4k30.bak /boot/firmware/cmdline.txt
sudo rm -f /etc/systemd/system/dex-loop.service.d/override.conf
sudo systemctl daemon-reload
'
# then physically swap the Dell back out for the Cam Link, and power-cycle.
# config.txt's comment block already documents both states; update its
# "currently holds" line back when you do this.
```
