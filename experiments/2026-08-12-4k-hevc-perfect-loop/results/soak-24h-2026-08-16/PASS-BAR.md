# M5 24h Soak — Pass Bar (pre-registered 2026-08-16, before the clock starts)

Written before any run data exists, so it cannot be adjusted after the fact. See
`STORY-4k-perfect-loop.md` (home-workspace) for full rationale; this file is the
pre-committed, load-bearing subset: identity of what's under test + the bar it
must clear.

## Run identity

- **dex-loop package:** `dex-loop_0.1.0-1_arm64.deb`, built by GitHub Actions run
  [31912269166](https://github.com/KTE/dex/actions/runs/31912269166), commit
  `5f8693bc3998fd4877edc40ee0d1645726727458` (branch
  `experiment/4k-hevc-perfect-loop`). Artifact sha256:
  `b6bd71367276faed51851828c5436e185a308be35688c02e8fd550bbe239fb58`.
  Installed via `sudo apt install ./dex-loop_0.1.0-1_arm64.deb` (the real path,
  not `dpkg -i`).
  - Caveat: `dex-loop --version` on the installed binary reports
    `dex-loop 0.1.0 (nogit)` — the CI build environment did not embed a git
    SHA into `build.rs`'s version string. Provenance for this run rests on
    the GitHub Actions run ID + artifact digest above, not the binary's own
    self-report. Worth fixing before M5 ships (a soak whose binary cannot
    identify itself is a smaller version of the exact problem this soak
    exists to close).
- **Asset under test:** `nts-1440p60-45M.265` (Naomi's *Zu Füssen Eschers Loop 1
  v1*, 45 Mbps encode), deployed as `/opt/dex/loop.265` +
  `/opt/dex/loop.265.json`. 2560×1440, 60000/1001 fps (59.94), HEVC Main
  profile, **High tier, Level 5** (`Main@L5@High`, `high-tier=1` in the x265
  encode log — comfortably inside High tier's ~100 Mbps L5 ceiling at a 45 Mbps
  VBV cap, unlike Main tier's 25 Mbps L5 / 40 Mbps L5.1 ceiling). sha256
  `d2eaee02b0ef9def3bf2b3f0f1bb84b03dc30d44fa31972d21472751150fc322`, verified
  against the real sidecar parser both pre-deploy and post-deploy
  (`make-sidecar.sh --check`, both `OK`).
- **Display target:** Dell U2719DC, 2560×1440@60 — the likely exhibition
  panel. `--mode 2560x1440@60` forced via a systemd drop-in
  (`/etc/systemd/system/dex-loop.service.d/override.conf`), because the
  packaged unit hardcodes `--mode 3840x2160@30` (the Cam-Link bench sink) —
  **a real M5 deployment gap this soak surfaces**, not a soak-only hack. The
  mode needs a proper config/sidecar home before ship.
- **cmdline.txt:** the Cam-Link-only `video=HDMI-A-1:3840x2160@30` force
  removed (backed up at `/boot/firmware/cmdline.txt.camlink-4k30.bak` on the
  Pi's SD card). The Dell's own preferred mode is correct unforced.
- **Pre-flight decode gate** (`scripts/measure-rate.sh`, run before this
  file was written): artwork `steady_ratio=1.000, overall_ratio=0.998,
  drops=0/0/0` — clears the known-good `dex-test-card-3s-2160p30-clouds`
  reference (`steady_ratio=0.954`) with margin. Both runs failed the script's
  strict `settle ≤ 20s` sub-gate (artwork 42.1s, reference itself 44.1s) —
  not discriminating since the known-good control fails it too under current
  bench conditions; steady-state ratio and zero drops are the metrics that
  carry weight here. (Required a small fix to `measure-rate.sh` first: the
  script's single immediate `playback-time` IPC query raced mpv's demuxer on
  this 212 MiB raw stream; patched to retry for up to 10s. Commit `ea1498a`.)

## Pass bar (any FAIL row = milestone NOT met)

Loop is 39.0 s → nominal 2214 wraps in 24h. Heartbeats: 144 nominal (600s
interval).

| # | Metric | Source | PASS | FAIL |
|---|---|---|---|---|
| 1 | Service restarts | `systemctl show -p NRestarts,ExecMainStartTimestamp` | 0, timestamp = boot | any restart |
| 2 | Boot count | `journalctl --list-boots` | exactly 1 boot ID in window | unplanned reboot caused by us |
| 3 | Heartbeat continuity | journal | ≥143 lines, no gap >700s | any gap |
| 4 | frame-drops (accumulated) | heartbeat | ≤5 at 24h, zero growth after first heartbeat | >5, or post-startup growth |
| 5 | vo-delayed (accumulated) | heartbeat | no growth across any contiguous 2h window after t+1h; value recorded as first-ever 24h baseline | sustained growth |
| 6 | Wraps | heartbeat | ≥2190 at 24h (nominal −1%) | below |
| 7 | pos / pos-age | heartbeat | pos advancing every line; pos-age <600s | frozen pos |
| 8 | RSS | telemetry TSV | slope t+1h→t+24h <0.5 MB/h AND total growth <25MB | steeper / more |
| 9 | Eyeball (start/mid/end) | human | landscape, smooth, no hang | sideways, stutter, hang |

Row 5 has no prior 24h baseline to compare against — this run's number becomes
the baseline for the next one, which is why it gets a trend bar instead of an
absolute one.

## VOID vs FAIL

VOID = corrupted environment measured, not the player. Fix, rerun; neither met
nor failed.

- Sticky throttle bits (`get_throttled` ≠ 0x0 at end) → VOID.
- Wrong mode negotiated at boot (Dell lands on anything but 2560x1440@60) →
  VOID; check within minutes of boot.
- Telemetry hole >10 min → VOIDs the RSS verdict only; heartbeat rows stand.
- External-cause reboot / power blip → VOID (panic/OOM reboot is FAIL, row 2).
- Manual interference (restarting the unit, touching `/opt/dex`, apt ops, T7's
  `--force-recovery-after-secs`) → VOID. Read-only ssh is fine.
- Dell auto-sleep/DPMS mid-run → VOID; disable in the OSD at setup.

## Status as written

Clock has **not** started. Phase 0 prep (this file's "Run identity" section)
is complete and verified on the Pi. The remaining step is physical and cannot
be done remotely: swap the Cam Link for the Dell U2719DC, disable its OSD
auto-sleep, confirm the case fan is running, then power on. See
`~/soak-24h/PHASE1-LAUNCH.md` on the Pi for the exact commands to run after
power-on, and the home-workspace sessionlog for full context.
