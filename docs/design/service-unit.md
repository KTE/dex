# The systemd unit

A dex player runs unattended for weeks with no operator, and the only way to switch it off is to cut the mains. This page is for a developer editing `dexd.service`: what each setting does, the display wait it calls, the maintainer scripts around it, and what breaks when a setting changes.

Terms are defined in [the glossary](../glossary.md); measured numbers and their conditions are in the [measurement record](measurements.md).

## Ordering and display ownership

The unit starts after `multi-user.target`, stops the console session on tty1 and waits for the assets directory `/opt/dex` to be mounted.

```
Conflicts=getty@tty1.service
After=multi-user.target
RequiresMountsFor=/opt/dex
```

A getty on tty1 holds DRM master, which would stop the player taking the display; `Conflicts=` stops the getty, so the player never fails with `device busy`.

`/opt/dex` is the assets directory, where the video, its sidecar, and the [exhibit config](exhibit-config.md) live. `RequiresMountsFor=` costs nothing while it is a directory on the root filesystem, and is what lets a card put the assets on their own partition without touching the unit: without it, dexd would start before that partition mounted and refuse.

A fresh device needs `sudo systemctl set-default multi-user.target`, so no desktop session claims the display.

## Restart policy

The unit relaunches the player two seconds after any exit, with no limit on the number of restarts.

```
Restart=always
RestartSec=2
StartLimitIntervalSec=0      # in [Unit]
```

Without `StartLimitIntervalSec=0`, systemd's default rate limit of 5 starts in 10 s puts the unit into a permanent `failed` state after a burst of crashes.

The key belongs in `[Unit]`: systemd accepted it in `[Service]` before version 229 and moved it there in that release. systemd does not report a key in the wrong section at load time, so the unit starts with the default rate limit in force. `systemd-analyze verify` catches that mistake, which is why it runs in [CI](ci.md) on this unit.

Do not use the restart loop to cover a missing dependency: add the `After=` or `RequiresMountsFor=` the player needs instead.

Escalating to a reboot when restarts do not restore playback is planned; see the [roadmap](roadmap.md).

## Service type

The unit is `Type=simple` and grants the main process systemd's notify socket.

```
Type=simple
NotifyAccess=main
```

A `Type=notify` unit that never sends `READY=1` stays inactive, and the player has no ready moment before its endless stream begins. `NotifyAccess=main` makes `WatchdogSec=` work under `Type=simple`: it grants the socket without the `READY=1` handshake.

## Start timeout

The unit waits for a display before starting the player, with a start timeout longer than that wait.

```
TimeoutStartSec=150
ExecStartPre=/usr/bin/dex-wait-hdmi
```

A projector can take 30 s to 90 s to present EDID while the Pi boots in about 15 s. `ExecStartPre` carries no `-` prefix, so a failed wait blocks the unit.

`TimeoutStartSec` must exceed the script's own wait, `DEX_HDMI_TIMEOUT`, which defaults to 120 s; raise `TimeoutStartSec` whenever `DEX_HDMI_TIMEOUT` is raised. Without a `TimeoutStartSec=` line, Debian's `DefaultTimeoutStartSec` of 90 s applies to `ExecStartPre`: systemd kills the script at about 90 s with a generic `start-pre operation timed out` message, before the script prints its own diagnostic, so a projector needing 91 s to 120 s never gets its full wait.

`ExecStartPre` runs before the main process is forked, and under `Type=simple` systemd starts counting `WatchdogSec=` at the exec of `ExecStart`. The display wait therefore consumes none of the 180 s watchdog budget (measured; see the [measurement record](measurements.md)).

## Command line

`ExecStart=/usr/bin/dexd` passes no arguments. What the player plays and how the display is driven come from `/opt/dex/exhibit.yaml` or `/opt/dex/exhibit.json`: `asset` names the video, `display_mode` and `kms_force` the display. The frame rate comes from the [sidecar](sidecar.md).

A `--mode` value or a positional asset path here is a cross-check: dexd refuses to start on any disagreement with the config, naming both values. See [Startup checks](startup-checks.md).

## Watchdog

systemd kills a player that is alive but no longer progressing, and the restart policy starts it again; `WatchdogSec=180` is the window.

dexd sends `WATCHDOG=1` once per health-check tick, about every 10 s, after that tick's recovery evaluation; the 600 s heartbeat never pings. The notify socket is non-blocking, so a full receiver queue is a dropped ping, not a retry. At 180 s, eighteen pings fit each window and about seventeen consecutive drops are needed before systemd kills the process (see the [measurement record](measurements.md)).

180 s exceeds the worst in-place recovery episode (about 2 minutes, derived), so a watchdog kill cannot preempt a recovery that would have finished.

A player that has stopped showing pictures gets no `time-pos` events, and two non-advancing health checks count as a stall. At most 3 in-place recoveries of about 20 s each follow, drawn from a budget that never refills; then dexd exits with code 1 (see [Failure handling](failure-handling.md)).

One path can hang: writing to standard error while the system log is not accepting writes. On that path the process stops sending pings rather than the ping call blocking.

Do not add a final ping to any exit path — the exit when the recovery budget is spent, an `END_FILE` event, and an mpv event-queue overflow. A ping sent just before that hang resets the `WatchdogSec=` countdown, and the hang goes undetected.

## User and sandbox

The player runs unprivileged: it reads `/opt/dex`, writes only its cache directory, and can open the display device.

```
User=dex
SupplementaryGroups=video render
ProtectSystem=strict
ProtectHome=yes
ReadOnlyPaths=/opt/dex
PrivateTmp=yes
NoNewPrivileges=yes
```

`dex` is a system user with no login, no password, and no home (`/nonexistent`), in the groups `video` and `render` only — what opening `/dev/dri` needs.

Under `ProtectSystem=strict` the filesystem is read-only apart from `/var/cache/dexd`, so the player never writes boot configuration. `dex-exhibit-apply(1)` is the separate step an operator runs as root after editing the exhibit config; deploys in this project are manual throughout.

## Cache directory

mpv needs a writable cache directory, and systemd provides one outside the `dex` user's home.

```
CacheDirectory=dexd
Environment=XDG_CACHE_HOME=/var/cache/dexd
```

`CacheDirectory=` makes systemd create `/var/cache/dexd` owned by `User=`, keep it writable under `ProtectSystem=strict`, and remove it on purge.

mpv resolves a shader-cache directory under `$XDG_CACHE_HOME`, falling back to `$HOME/.cache`, during video-output init and before it consults `--gpu-shader-cache`; setting that flag to `no` changes nothing on mpv 0.40.

Without these two lines the service logs `Failed to create /nonexistent for shader cache` on every start. Keep both lines: the system log is the only diagnostic channel on a deployed player, and an error printed on every healthy start makes it useless.

## Shutdown

The unit stops the player with `SIGTERM` and waits five seconds.

```
KillSignal=SIGTERM
TimeoutStopSec=5
```

There is no graceful shutdown; the player is built to survive the mains being cut. The kernel releases DRM master when the process exits, so the next start acquires it cleanly.

## dex-wait-hdmi

The script polls `/sys/class/drm/card*-HDMI-A-*/status` once per second until one connector reads `connected`, then exits 0; the wait lasts 120 s by default, and `DEX_HDMI_TIMEOUT` overrides it. The script globs the card number, because vc4 and v3d probe order makes `card0` and `card1` unstable across kernel versions.

It installs to `/usr/bin`, like `dexd` and `dex-exhibit-apply`, because Debian policy reserves `/usr/local` for the local administrator.

On timeout the script prints `dex-wait-hdmi: no connected HDMI connector after 120s` to standard error, naming the timeout in force, and exits 1. The unit then fails, and `Restart=always` retries the whole sequence instead of starting the player into a display that is not there.

The wait is a fallback; the primary fix is at the KMS layer. A forced display mode whose `video=` token ends in `D` makes the connector read connected before a display is attached, so the Pi keeps the forced mode and the projector locks on when it warms up. `dex-exhibit-apply(1)` writes that token into `/boot/firmware/cmdline.txt` from `kms_force`.

`dex-exhibit-apply(1)` writes the `video=` token only. An operator adds the `drm.edid_firmware=` part by hand on the same line, naming a saved copy of the display's EDID under `/lib/firmware`:

```
drm.edid_firmware=HDMI-A-1:edid/dex.bin video=HDMI-A-1:3840x2160@30D
```

Capturing the EDID file is an install-time step; see `dex-wait-hdmi(1)`. Even with a forced mode, the wait covers a swapped cable and a replaced projector whose saved EDID no longer fits.

## Maintainer scripts

`postinst` creates what the unit requires and cannot create itself, inside a `case "$1" in configure)` block so it runs only on the configure action:

- the `dex` user, if `getent passwd dex` finds none;
- `/opt/dex`, root-owned and mode 0755, since the unit mounts it read-only and the player only reads.

The group loop adds `dex` to `video` and `render`, skipping a group the system lacks. Adding a user to a group it is already in is a no-op, so re-running the script repairs dropped groups.

The package installs no exhibit config and creates no `/etc/dex`. `postinst` creates no config; it reports three things without failing the install:

- neither `/opt/dex/exhibit.yaml` nor `/opt/dex/exhibit.json` exists — the message names the three files a player needs (video, sidecar, exhibit config) and a minimal config;
- a config sits under `/etc/dex`, which dexd no longer reads; the package leaves it, since it must not delete a file an administrator created;
- a drop-in under `/etc/systemd/system/dexd.service.d` still passes `--mode`, left for the same reason.

A disagreeing drop-in already makes dexd refuse, naming both values. The message covers one whose value agrees: the player starts, and where the display mode came from is no longer visible.

`postrm` keeps the `dex` user and `/opt/dex`: the video is operator content the package never shipped, and the user may be named in something an operator wrote. Debian policy permits keeping system users.

The package metadata sets `enable = true`, which hooks the unit onto `[Install] WantedBy=multi-user.target`, and sets `start = false`. A device that is only power-cycled comes back playing, and an operator picks the moment the unit takes DRM master. [Packaging](packaging.md) covers the rest.

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| `Type=notify` | The unit stays inactive | The player sends no `READY=1` |
| `StartLimitIntervalSec` in `[Service]` | systemd ignores the key | The default rate limit stays in force |
| Debian's default start timeout | systemd kills the wait at about 90 s | `dex-wait-hdmi`'s 120 s diagnostic never prints |
| A writable home for the shader cache | The player writes outside its cache directory | `CacheDirectory=` gives one systemd creates and removes on purge |
| Boot-config writes inside the player | The player runs as root | `dex-exhibit-apply(1)` does it as a separate step |
| Removing the `dex` user on purge | Frees a passwd entry | May break an operator's own unit or cron job |
| Starting the unit during install | Playback begins at once | The unit would take DRM master from the console session |
