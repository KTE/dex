# Failure handling

This page describes how a running dexd notices that it has stopped showing pictures, what it does about it, and which failures it cannot see. It is for a developer reading `main.rs`, `health.rs`, `heartbeat.rs`, and `watchdog.rs`.

Terms are defined in [the glossary](../glossary.md); measured numbers and their conditions are in the [measurement record](measurements.md). The refusals that happen before mpv exists are on [Startup checks](startup-checks.md), and the unit settings named here are explained line by line in [The systemd unit](service-unit.md).

## How a fault is answered

Four things answer a fault, in order; two of them are built.

| Layer | Mechanism | Recovers |
|---|---|---|
| In-place recovery | dexd re-issues `loadfile` on its own stream and keeps running | a stalled demux or decode chain |
| Process restart | `Restart=always`, `RestartSec=2` in the unit | anything the process cannot repair in place |
| Reboot escalation | planned | state that a restart does not clear, such as a stuck DRM device |
| Hardware watchdog | planned | kernel hangs and total lockup |

When one does not recover the player, the next runs. `Restart=always` with `StartLimitIntervalSec=0` never gives up, so the process comes back, whichever of them repairs the fault.

The health check, which drives the first of them, runs on the supervisor thread, off the decode path, and never blocks presentation (decided).

## Fatal events

An `END_FILE` event from mpv is fatal, because dexd's stream has no end. The read callback wraps to byte 0 instead of reporting end of file (see [Why an endless stream](endless-stream.md)), so playback ending means something failed. dexd logs the reason and calls `exit(1)`:

```
dexd: fatal: playback ended (reason=2, error=success) -- an endless stream must never end; exiting so systemd restarts this process
```

dexd absorbs one kind of `END_FILE`: the one its own recovery causes ([below](#expected-end-of-file)).

dexd creates its mpv handle with idle mode on, so libmpv emits `END_FILE` for a failed load and stays idle, never emitting `SHUTDOWN`.

Display-side failures also arrive as `END_FILE`. With mpv's normal video output, `vo=gpu`, mpv creates the video output during the file load, not during handle setup. A projector that is not awake, a connector with no EDID, or a getty still holding DRM master therefore pass both the handle-setup and the `loadfile` return codes and land as `END_FILE`.

`MPV_EVENT_QUEUE_OVERFLOW` is fatal too. mpv's internal event ring fills at 1000 pending events and drops every event after that — possibly an `END_FILE` — until the client drains it, and reserves no slots for fatal events. What was lost cannot be known, so dexd exits.

dexd exits with `std::process::exit(1)` on a fatal event and on an exhausted recovery budget, never through the shared `mpv_terminate_destroy` teardown. Only a genuine `SHUTDOWN` breaks the loop and tears down normally. A rejected mpv option is an operator error and exits 2 instead (see [Startup checks](startup-checks.md)).

Removing the end of file also removes the point at which mpv detects many failures: a stream that probes as HEVC but never yields a decodable frame can leave mpv buffering with no event at all (assumed). The health check notices that.

A test forces a display-free failure (`vo=null` with every track deselected, so mpv reaches "nothing to play") and asserts exit code 1 within a 30 s deadline; a player still running when it expires is killed and the test fails, so a hang on a failure path cannot pass.

## Health check

The health check judges whether playback is advancing, using only values mpv pushes. dexd registers one `mpv_observe_property` call for `time-pos` at startup, which does not block. It then reads the position out of `MPV_EVENT_PROPERTY_CHANGE` events on the same wait loop that detects the fatal events. No synchronous property read exists in the runtime path.

That also makes an unresponsive mpv core visible: when the core stops, no property-change events arrive, and the absence is the stall signal.

Every 10 s, one tick feeds the latest known position to the policy in `health.rs`:

- Two consecutive ticks whose position does not strictly increase count as a stall; one does not, which absorbs jitter around a check boundary. Detection latency for a real stall is therefore about 20 s (derived).
- A position that goes backwards — a property glitch during a video-output reconfigure — counts as a stall, because progress requires a strictly greater position.
- The first sample after start or after a recovery counts as progress. mpv's startup latency before a 4K decode begins is a few hundred milliseconds to a few seconds (assumed), and that grace keeps it from reading as a fault.
- A tick with no sample counts as a stall and gets no first-sample grace, so a startup that never produces a position is still caught after two ticks.

`mpv_wait_event` uses that cadence as its timeout. In healthy playback mpv delivers position events often enough to wake the thread on its own. During a stall no events arrive at all, and the timeout guarantees the tick still happens. The wake costs one event drained and two numbers compared, on the supervisor thread.

If the `time-pos` subscription fails to register, dexd creates no health monitor, skips the tick, and warns once that in-place recovery is off for this run and process restart still applies. Without that guard the position would stay unknown forever, every tick would read as a stall, and dexd would issue recoveries against a healthy player and eventually exit.

## In-place recovery

dexd answers a qualifying stall by re-issuing `loadfile loop://endless replace` on its own stream and continuing. The command goes through `mpv_command_async`, never the blocking `mpv_command`, used once at startup, because the call runs on the supervisor thread that must keep detecting fatal events.

A `loadfile ... replace` tears down and rebuilds the demuxer and the decoder chain, and forces a video-output reconfigure. It does not tear down the video output itself: mpv tears the video output down only at process termination (documented in mpv 0.40's source). A fault in the DRM or GPU context can therefore survive the replace and still need a process restart.

`MPV_EVENT_COMMAND_REPLY` is diagnostic; nothing gates on it. The next tick judges the recovery by whether the position starts advancing again, and a logged rejection makes that judgement traceable afterwards.

The reload re-opens the stream from byte 0, so mpv's position counter restarts near zero. Both the policy and its driver forget the pre-recovery position when the attempt is issued. Keeping it would make the next sample read either as a continuing stall or as a fresh first sample that stretches the escalation timeline.

### Recovery budget

Attempts come from a budget of 3 for the whole process lifetime, cumulative across separate stall episodes and never refilled. Three bounds the worst case and still absorbs a handful of isolated glitches over a multi-week run.

When the budget is spent, the next qualifying stall escalates: dexd logs the exhausted budget and the reason, then exits 1.

dexd clears the consecutive-stall counter before each attempt or escalation, so the next judgement gets its own two-tick window; the budget is untouched by that reset.

### Expected end-of-file

`loadfile ... replace` makes mpv emit `END_FILE` with reason `stop` (value 2) for the file being replaced (measured on mpv 0.40.0 on a Raspberry Pi; see [measurements.md](measurements.md#the-event-an-in-place-recovery-produces)). dexd absorbs that event and continues:

```
dexd: health check: in-place recovery's loadfile replaced the stream; absorbing the expected END_FILE(reason=stop) for the file it replaced (0 more still outstanding), not treating it as a failure
```

The absorption test is a pure function of two values: a pending count above zero and reason `stop`. Every other reason stays fatal, and so does a `stop` with nothing pending — the case in the fatal line above. Nothing else in the program issues a command that produces a stop-reason end-of-file, so the count tells dexd's own teardown apart from a failure carrying the same code.

The pending value is a count. The health-check tick and the forced-recovery probe can each queue a recovery in the same loop iteration. `mpv_command_async` only queues against a core that may still be busy, so two recoveries can be in flight, each producing its own stop event. A flag would absorb the first and treat the second — the teardown of a recovery that just worked — as fatal, killing a healthy-again process.

The count is incremented only after the command is queued, because only then is a stop event coming. It is decremented by one on each absorbed stop or on a rejected command reply, which keeps a second attempt's stop expected.

mpv exposes no runtime name lookup for end-of-file reasons, so the stop reason's value is transcribed by hand. A wrong value fails in the safe direction: the absorption never matches and the stop falls through to the fatal path.

## Heartbeat

The heartbeat is one line in the system log every 600 s, frequent enough to bound when a player died to a useful window:

```
dexd: heartbeat loops=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=2 pos=3599.4s pos-age=0s watchdog=armed pings-dropped=0
```

The heartbeat never calls into mpv: the function that emits the line has no mpv handle, and `mpv_get_property_string` and `mpv_free` are absent from the FFI surface, so the call cannot be written. A synchronous property read waits on a condition variable with no timeout until mpv's core thread reaches its dispatch loop (documented in mpv 0.40's source). A core stuck in a display call against a projector that has stopped responding never reaches that point.

dexd learns every value the line prints from an event, which is safe on the supervisor thread. Observed-property getters run on the core thread with the client lock dropped, so a stuck getter blocks neither the supervisor thread nor `mpv_wait_event`: an unresponsive core delivers no further events.

mpv generates property-change events inside `mpv_wait_event` once the queue has drained and never queues them, so observing more properties cannot push dexd toward the fatal queue overflow.

An observer costs two events per counter around startup: an initial notification with no value, then the first real value once the video-output chain exists. After that an event arrives only when the value changes.

mpv's drop counters are per playback session and restart at 0 when a recovery rebuilds the chain, so dexd accumulates. It adds forward deltas and reads any decrease as a session reset whose post-reset value is new, so `frame-drops=0` cannot be a false all-clear after a recovery.

A counter reads `n/a` until its first value, and `off` when the subscription never registered. `mpv_observe_property` never validates a property name, so a rename upstream would subscribe cleanly and sit at `n/a` for the whole run. When mpv reports a counter as unavailable — no video-output chain at startup, or during a recovery's teardown — dexd clears the diffing baseline and keeps the total already earned.

The position uses a fixed one-decimal format, so a position from weeks of uptime stays readable (three weeks reads `pos=1814400.0s`), and `pos-age=` separates a healthy player from one that has stopped reporting. The chip temperature comes from `/sys/class/thermal/thermal_zone0/temp` in millidegrees; where the kernel does not expose it, the field reads `n/a`.

The first heartbeat follows the `loadfile` and proves temperature reading and line formatting on every boot. It does not prove the subscriptions: the counters and the position read `n/a` there because nothing has decoded. During the long-running test on a Raspberry Pi 4, every heartbeat from t+600 s carried numeric counters and `pos-age=0s` (measured; see the [measurement record](measurements.md)).

## Systemd watchdog

The watchdog covers a hazard neither the health check nor the heartbeat can see: dexd's own supervisor thread hanging in code that is not an mpv call. The canonical case is a log write blocking against a system log that has stopped responding, including on the escalation path that exits so the service manager can take over. Detecting that needs an external actor.

The unit carries `WatchdogSec=180` and `NotifyAccess=main`, and `Type` stays `simple`. A notify unit that never sends `READY=1` sits inactive forever, and dexd has no ready moment before the endless stream starts, so `READY=1` is never sent. `STOPPING=1` is never sent either, because there is no graceful shutdown.

The 180 s window exceeds the worst case of a full in-place recovery episode (about 2 minutes, derived), so a watchdog kill cannot pre-empt a recovery that would have finished.

dexd sends `WATCHDOG=1` once per 10 s tick, after that tick's evaluation and any recovery command have completed, and from nowhere else.

That ordering means a broken player stops pinging. Because the budget never refills, a player whose display has stopped responding delivers no further events, stalls, recovers at most three times, and exits. That is a bounded number of pings, then either an exit the restart policy handles or, on the one path that can still hang, no more pings.

The ping is sent even when the health check is disabled for the run. It then certifies only that the loop completed an iteration, and the startup warning says so.

### Ping protocol

The whole wire protocol is the literal bytes `WATCHDOG=1`, with no trailing newline, sent to `$NOTIFY_SOCKET` over an AF_UNIX datagram socket. It is written by hand against `std` alone — `UnixDatagram` covers the abstract-namespace case too — and adds no crate to the package.

Unix datagram sockets have flow control, so a blocking send against a full receiver queue would block the supervisor thread. The socket is opened once, in non-blocking mode, and held for the life of the process. Any send failure — a full queue, a socket path that vanished — counts as a dropped ping, never retried inline, never a panic, and surfaces in the heartbeat as `pings-dropped=`.

The handshake resolves once at startup, before the first heartbeat, so that line already carries the real state. dexd logs each inert case once rather than warning:

- No `$NOTIFY_SOCKET`, or an empty one: inert. This is the common case — a development machine, a test session, CI, any invocation off systemd.
- `WATCHDOG_PID` set and not this process: inert, because pinging under another identity would be wrong. A value that does not parse is treated the same way.
- Otherwise armed, with the window taken from `WATCHDOG_USEC`. A window shorter than twice the tick cadence produces a warning, and pings continue regardless; 180 s against a 10 s cadence gives 18 pings per window and no warning. An unparseable value arms with no window and no invented warning.

Setup itself can fail — address resolution, socket creation, setting non-blocking mode. When `$WATCHDOG_USEC` is present, systemd's kill timer is already running, whatever dexd logs. dexd exits 1 and lets `RestartSec=2` retry. Running without pings under an armed timer means a kill every window: the screen goes black every 3 minutes under the shipped unit (derived).

With no timer armed, dexd logs once and runs without pings. The pid-mismatch case adds a warning about the coming kill loop when the timer is armed, so the log explains the restarts that follow.

Never add a final ping to an exit path. A ping sent just before a hang on that path would reset the countdown.

A watchdog kill is recovered by `Restart=` like an exit-code failure, confirmed on a Raspberry Pi (measured), so it needs no special handling. When reboot escalation is built, a watchdog-caused restart should count toward its window like any other.

## Test-rig probes

Two flags force a failure on purpose. Both require `--test-rig-no-sidecar`, are refused without it, print a loud warning at startup, and appear in no deployment.

**`--test-rig-force-recovery-after-secs N`** forces the same recovery decision a real stall produces, once, drawing from the same budget and performing the same baseline reset. The health-check tick and the probe route through one function, so the probe drives the identical mpv-facing mechanics; only the log prefix differs. It fires N seconds after the `loadfile` request is queued, not after N seconds of confirmed playback, so a small N can fire during decode startup.

**`--test-rig-hang-after-secs N`** parks the supervisor thread forever N seconds after startup, reproducing a hang outside any mpv call. It is checked before the event dispatch, so a run that reaches a fatal event immediately cannot win the race, and it never resumes: only an external actor ends the process. Run under a temporary unit with a short `WatchdogSec=`, it produces a watchdog timeout, a `SIGABRT`, exit status `6/ABRT`, and a restart in the system log — the procedure is in [Building and testing dexd](development.md).

The automated tests stop short of the picture. CI runs the forced recovery against a real mpv under software decode with `vo=null` ([Continuous integration](ci.md)). It proves that the process survives its own recovery, that the position resumes advancing, and that no second recovery fires.

It does not prove that the picture returns on hardware: `hwdec=drm`, `gpu-hwdec-interop=drmprime-overlay`, and the plane assignment are all skipped in a container with no DRM device and no GPU. Two things a container cannot reach are verified by hand on a Raspberry Pi: the picture after a recovery, and the kill after a hang.

## Diagnostics

libmpv discards every diagnostic it produces unless the client asks for it: `terminal=no` is its default, so log output goes nowhere unless requested as events. dexd requests level `warn` and forwards each message as `mpv/<prefix>: <text>`. A failed log request or property registration warns and is never fatal, because the health check and the counters are diagnostics on top of a working player.

Every failure class reaches the system log and nothing else. There is no getty on tty1, conflicted away so the player can take DRM, so at a venue every failure looks the same: a black screen. An on-site fault signal is listed in the [roadmap](roadmap.md).

## Device-level failures

Some failures belong to the device, not to the running player.

- A Raspberry Pi that boots before its display is awake reads no EDID and lands on a 1024×768 fallback, and the console does not re-set the mode once the display appears. Players are switched off at the mains, so this is the normal case; the fix is a forced display mode in `cmdline.txt`, covered in [Exhibit config](exhibit-config.md).
- Without `hwdec-software-fallback=no`, a decoder that cannot reach the hardware path falls back to software with no error and plays 3840×2160, 30 fps at about 14 fps (measured). dexd makes that fatal, turning an invisible collapse into an `END_FILE` it can restart from.
- Power is cut at the mains with no graceful shutdown. Existing installations have survived that cycling on the current image arrangement (decided), so dexd does not require a read-only root filesystem. That evidence comes from dexOS images; a dexd card is plain trixie plus the .deb, so a card that comes back corrupt is a reason to revisit it.
- A mains cut during an in-place write of `cmdline.txt` would leave it truncated and the Raspberry Pi unbootable. `dex-exhibit-apply` therefore writes a temporary file, fsyncs it, renames it over the original, and fsyncs the directory (see [Exhibit config](exhibit-config.md)).
- Boot output stays visible: no quiet boot (decided).

## Residual gaps

| Gap | State |
|---|---|
| Signal-level failure: HDMI signal lost, panel powered off, plane presenting to a disconnected display | Out of scope. The position keeps advancing, the health check reads healthy, pings continue, and the display stays black. Polling DRM connector status from the health tick — the same sysfs files the mode pre-flight reads — would close it. |
| Two overlapping recoveries | Tests enforce that both stop events are absorbed; the count path has not run on a device, because nothing recovered during the long-running test. |
| The health check disabled for a run | The ping then certifies only that the loop iterates, so a player whose display stops responding pings forever. The startup warning is the only trace. |
| Whether in-place recovery repairs a DRM or GPU fault | Not tested. A physical HDMI-loss test on a Raspberry Pi would settle it. |
| Startup grace on a slow display | Not measured. A display needing longer than the two-tick window (about 20 s) for a first position sample would draw a recovery mid-startup. 4K decode startup runs 1–3 s behind `dex-wait-hdmi`'s wait for the display (measured). |
| The "no second recovery" assertion in the forced-recovery test | It needs the loop still ticking; a loop that stopped iterating right after the absorb would satisfy every assertion with nothing running. A per-tick liveness line while the probe is armed would close it. |
| Drop-counter under-count across a recovery | Narrowed, not closed. mpv coalesces property events, so a teardown, a restart at 0, and a climb past the old total between two drains can hide the decrease. |

## Alternatives

| Option | Outcome |
|---|---|
| Wait only for `SHUTDOWN` and ignore `END_FILE` | Rejected: libmpv idles instead of exiting, so a failed load leaves the process alive with a black screen. |
| `break` into the shared mpv teardown on a fatal event | Rejected: `mpv_terminate_destroy` joins mpv's threads and can block on the hang it is trying to escape. |
| Read properties synchronously for the heartbeat | Rejected: the read hangs on the fault it reports. |
| Refill the recovery budget after a healthy period | Rejected: a flapping fault would reset the counter before it reached the cap, leaving the retry total unbounded. |
| `StartLimitAction=reboot` for reboot escalation | Not used: it interacts badly with `StartLimitIntervalSec=0`. A second unit triggered by `OnFailure=` is the planned shape. |
| A libsystemd binding or the sd-notify crate for the ping | Rejected: a binding adds a shared-object link to the package's derived dependencies, and a crate leaves the ping policy, the handshake, and the non-blocking audit here anyway. |
| Stop pinging when the health check is disabled | Rejected: it turns a degraded but working run into a guaranteed kill every window. |
| Suppress the second recovery when one is already in flight | Rejected: absorbing both stop events is simpler than preventing the overlap. |
