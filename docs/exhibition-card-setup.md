# Building an exhibition card — cheat sheet

For a Raspberry Pi 4 playing one artwork on one display, unattended, switched off
at the mains. Current as of 2026-08-17 (dex-loop 0.1.0, Debian trixie).

**Division of labour:** Max does §1 (physical: flash, boot, cable up). Everything
from §2 is remote and the agent can do it. Hand over by saying the hostname and
which display is attached.

---

## 1. What Max does

### 1.1 Flash

Raspberry Pi Imager → **Raspberry Pi OS Lite (64-bit)**, Debian **trixie**.
Lite, not Desktop: a desktop session takes DRM master and the player then cannot
have the display.

In Imager's settings (the gear), set **all** of these — they are what make the
card reachable without a keyboard:

| Setting | Value |
|---|---|
| Hostname | `dexpi4` (or the next free `dexpiN`) |
| Username | `dexpi` |
| SSH | **enabled**, public-key only, paste `~/.ssh/id_ed25519.pub` |
| Wi-Fi | SSID + password, **country CH** |
| Locale/timezone | Europe/Zurich |

### 1.2 Boot and cable

Boot it once and let it settle (~2 min: it resizes the filesystem and reboots).
Connect the display **before** first boot if possible — EDID read at boot is what
gives the connector its mode list.

### 1.3 Hand over

Tell the agent: **hostname**, and **which display** is attached (this decides the
mode, and the two known panels need opposite settings — see §3).

Sanity check that it is reachable at all:

```bash
ssh dexpi@dexpi4.local 'hostname; uptime -p'
```

---

## 2. What the agent does

Roughly 20 minutes, mostly waiting on apt.

```bash
# 2.1 Base
sudo apt-get update && sudo apt-get -y full-upgrade
sudo apt-get install -y --no-install-recommends libmpv2 edid-decode libdrm-tests tmux
sudo systemctl set-default multi-user.target      # no desktop, nothing else may own DRM
sudo loginctl enable-linger dexpi                 # or tmux dies with the last ssh session

# 2.2 Install the player from a CI build (never a local build for a real card:
#     the package must be the artifact CI produced and asserted)
gh run download <RUN_ID> --repo KTE/dex --name dex-loop-deb
scp dex-loop_*.deb dexpi@<host>:/tmp/
ssh dexpi@<host> 'sudo apt-get install -y /tmp/dex-loop_*.deb'

# 2.3 PROVE it is the intended build. --version writes to STDERR.
ssh dexpi@<host> 'dex-loop --version 2>&1 | head -1'   # must show a real commit, never (nogit)
```

Then the asset and the display mode — see §3 and §4 — and finally §5.

---

## 3. The display mode (the part that bites)

**The mode is per-display and the two known panels need opposite settings.**
Getting it wrong is silent: the Pi transmits a signal the sink cannot show, and
it looks exactly like a player fault.

| Display | `cmdline.txt` | Why |
|---|---|---|
| **Elgato Cam Link 4K** (bench capture) | needs `video=HDMI-A-1:3840x2160@30` | It advertises 4K30 as its *preferred* timing and `vc4` still builds **zero** 4K modes from it. Forced, the identical 297 MHz timing works. |
| **Dell U2719DC** (2560×1440) | must **NOT** have it | Its EDID never mentions 2160. Forcing 4K sends a signal it cannot display. Its own preferred `2560x1440@59.95` is correct unforced. |

So **removing that line is a change of target, not cleanup** — a mistake made and
reverted inside four hours on 2026-08-15.

Check what the connector actually offers before deciding:

```bash
ssh dexpi@<host> 'edid-decode /sys/class/drm/card1-HDMI-A-1/edid | grep -i "Display Product Name"
                  modetest -M vc4 -c | grep -m3 "^  #"'
```

> **This section is a stopgap.** F6 replaces it with an **exhibit `dex.yaml`** that
> names the asset and the mode in one human-editable file. Once F6 lands, prefer
> that and treat this table as the fallback.

---

## 4. Asset

The player **refuses to start without a sidecar** (F3) rather than guessing a
frame rate — a wrong guess plays slow forever with every metric green.

```bash
scp artwork.265 dexpi@<host>:/tmp/
ssh dexpi@<host> 'sudo install -m644 /tmp/artwork.265 /opt/dex/loop.265'
# generate + verify the sidecar with the crate's REAL parser, not by hand
ssh dexpi@<host> 'cd ~/dex/experiments/2026-08-12-4k-hevc-perfect-loop &&
                  ./scripts/make-sidecar.sh /opt/dex/loop.265 &&
                  ./scripts/make-sidecar.sh --check /opt/dex/loop.265'
```

Ingest requirements, if the asset is being encoded fresh:

- raw **Annex-B** `.265`, no container, no audio
- **closed GOP with an IDR at frame 0** (`keyint=min-keyint`, `no-open-gop=1`,
  `scenecut=0`) — the endless-stream trick is only valid because of this
- **capped VBR** (`-crf` plus `-maxrate`/`-bufsize`): a fixed-function decoder is
  bounded by *peak* demand, so unconstrained VBR can spike and stall a decoder
  that was coping a moment earlier
- `-noautorotate` if the source carries a rotation matrix — raw Annex-B has no
  container to hold it, and ffmpeg silently bakes in a rotation otherwise

---

## 5. Verify before leaving

```bash
ssh dexpi@<host> '
  systemctl is-enabled dex-loop            # enabled -> comes back after a mains cut
  sudo systemctl start dex-loop
  sleep 25
  systemctl is-active dex-loop             # active
  sudo journalctl -u dex-loop -o cat | tail -5
'
```

Look for, in order:

1. `dex-loop 0.1.0 (<commit>)` — a real hash, never `(nogit)`
2. `fps <rate> (sidecar)` — **`(sidecar)`**, not a bench override
3. `Using HW-overlay mode` — the zero-copy KMS path; without it decode misses realtime
4. After ~10 min: `heartbeat ... frame-drops=0 vo-delayed=0`

Then **power-cycle it at the wall** and confirm it comes back playing on its own.
That is the actual operating condition, and it is the only test of it.

---

## Known gaps (2026-08-17)

- **F6** — the mode lives in `cmdline.txt` and a systemd drop-in rather than a
  config file. In progress.
- **F10** — no systemd watchdog yet: nothing acts if the player stops being
  healthy without exiting. In progress.
- **F8** — every failure is journal-only, so on site a fault photographs as a
  black rectangle. Post-1.0.
- **Same-version packages do not upgrade.** `apt` skips an identical version
  silently; CI now stamps `0.1.0-<run>+g<sha>`, but a hand-built `.deb` still
  needs `--reinstall`.
