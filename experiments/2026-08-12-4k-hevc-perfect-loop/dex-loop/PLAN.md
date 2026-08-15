# dex-loop hardening plan

Status: draft, 2026-08-15. Written after three adversarial reviews and two days
of bench work. Supersedes nothing; the player works and is measured seamless —
this is what stands between "works on the bench" and "runs in a gallery for
weeks with nobody watching".

## Design principles

Recorded because they decide the trade-offs below, and because two of them are
in tension.

1. **Correct playback is the bar.** Realtime rate, full frame rate, correct
   colour and geometry. Seamlessness is one clause, not the whole thing.
2. **Fail loudly, never silently.** The worst outcome is not a crash — it is a
   live process showing a black wall while every metric reads green. Every
   silent-degradation path found so far (idle-hang, software-decode fallback,
   discarded logs, wrong `--fps`) is a bug even when playback "works".
3. **Resilience and self-healing, in escalating tiers** (Max, 2026-08-15). The
   machine has no operator. So it must try to fix itself, and each tier must
   escalate when the one below it fails:

   | Tier | Mechanism | Recovers from |
   |---|---|---|
   | 0 | in-process periodic health check → re-init the VO/display | display not ready at boot; intermittent HDMI loss; a sink that woke up late |
   | 1 | exit non-zero → `Restart=always` | anything the process cannot repair in place |
   | 2 | repeated restarts within a window → **reboot** | state the process cannot clear (wedged DRM, driver fault) |
   | 3 | hardware watchdog → reboot | kernel hangs, total lockup |

   **This is in deliberate tension with principle 1.** A health check costs
   cycles on a device with a hard per-frame budget. The resolution: checks run
   at low frequency (order 10 s), off the decode path, and must never block
   presentation. A check that could stall a frame is worse than the fault it
   looks for.
4. **The mains switch is the shutdown path.** Do not implement graceful
   shutdown; make abrupt death safe instead.
5. **The interface must make operator error impossible**, not merely detectable
   — because there is no operator to detect it. See F3.

## F — Fixes outstanding

Numbered so tests can cite them. Findings already fixed (END_FILE hang,
`user_data` stack pointer, `read_fn` zero-return, sentinels, log capture,
software-fallback, `panic=abort`, `Send`) are not repeated here.

### F1 — Tier-0 self-healing: periodic health check with VO re-init
**Why:** the review's top finding was that a display absent at startup yields a
permanent black screen. Making it fatal (done) plus restart (F2) covers the boot
case, but not an *intermittent* loss mid-show, where restarting a healthy player
is heavy-handed and loses a few seconds of picture.
**What:** a low-frequency supervisor thread that samples `time-pos` (or
`estimated-frame-number`); if it has not advanced across two consecutive checks,
attempt in-place recovery (`loadfile` again → forces VO reconfiguration) before
escalating to exit. Cap in-place attempts; escalate to tier 1 after N.
**Cost control:** one property read per 10 s on a non-decode thread.

**2026-08-15, implemented:** landed as `src/health.rs` (pure escalation policy,
`HealthMonitor`/`HealthAction`, 11 unit tests — attempts, thresholds, the
cumulative-budget-never-refills property, the position-baseline reset a real
`loadfile` restart requires) plus the mpv-facing wiring in `main.rs`. One change
from this text: **not** a synchronous property read on a dedicated thread — see
F9 above, whose SUSPECTED core-wedge concern this design takes as the deciding
constraint rather than an open question. `time-pos` is subscribed once via
`mpv_observe_property` and sampled only from `MPV_EVENT_PROPERTY_CHANGE` events
on the existing (already-proven-non-blocking) event-loop thread; recovery is
issued via `mpv_command_async`, never the blocking `mpv_command`. No new
synchronous mpv call is introduced anywhere in this feature. The event-wait
timeout was tightened from 30 s to `HEALTH_CHECK_SECS` (10 s) so a genuine stall
— which by definition produces no events — still gets evaluated on schedule.
Verified on the Pi: full suite green (58 → 74 tests) and a ~75 s manual run of
the real `loop4k.265` asset against the real DRM display, health check active
and silent throughout (~7 ticks, zero spurious recovery/escalation).

**2026-08-15 addendum, from three further adversarial reviews (rust/libmpv/gallery-ops
lenses, run independently, all three CONFIRMING the same root cause via live
mpv 0.40 probes on the Pi):**

- **C1/CRITICAL — fixed.** In-place recovery's own `loadfile ... replace`
  killed the process on its first step: mpv v0.40 delivers
  `END_FILE(reason=stop)` for the file being replaced (bench-confirmed live
  by all three reviews independently), and the event loop treated EVERY
  end-file as fatal. So the ONE untested path (the Pi soak never saw a real
  stall, hence "silent throughout" above) was the one that was broken --
  every recovery attempt would have killed itself before doing any good,
  silently converting tier 0 into an unconditional tier-1 restart with a
  misleading journal line ("playback ended" instead of "recovery
  completed"). Fixed: `MPV_END_FILE_REASON_STOP` added to `ffi_consts.rs`
  (2, bench-verified); `main.rs` now sets a `recovery_stop_pending` flag
  when (and only when) the recovery's `mpv_command_async` is actually
  queued, clears it either on consuming the matching END_FILE(stop) or on
  an async command-reply error (no END_FILE will come for a rejected
  command), and absorbs exactly one matching stop instead of exiting. Pure
  decision logic (`is_expected_recovery_stop`) extracted and unit-tested
  (3 new tests) so this exact regression -- the entire fix is one boolean
  condition -- cannot silently reappear.
- **Baseline-reset bug — fixed.** `main.rs`'s driver-side `last_position`
  was never cleared when a recovery was issued, so a wedged core's stale
  pre-recovery position could read as a false "first sample = progress"
  tick, stretching the worst-case escalation timeline beyond the documented
  ~2-tick window. Now cleared in the same arm that issues the recovery,
  mirroring `HealthMonitor::tick`'s own baseline reset.
- **False "DISABLED" warning — fixed.** If `mpv_observe_property(time-pos)`
  ever fails to register (OOM/unsupported format per client.h -- very
  unlikely, near-zero probability), the code printed "tier-0 self-healing is
  DISABLED" but then kept running the tick loop anyway with `last_position`
  permanently `None` -- every tick would read as a stall, eventually issuing
  recovery commands (and, once the budget exhausted, exiting) against a
  perfectly healthy player. `health` is now `Option<HealthMonitor>`, `None`
  when registration failed, and the entire tick block is skipped in that
  case -- the warning is now actually true.
- **Escalate/fatal-exit path — fixed.** All three fatal exit sites
  (Escalate, END_FILE, QUEUE_OVERFLOW) previously `break`-ed into a shared
  `mpv_terminate_destroy(ctx)` call before returning. `mpv_terminate_destroy`
  synchronously joins mpv's own threads -- on the Escalate path specifically,
  that fires precisely because the core LOOKS wedged, so the one exit path
  whose entire job is "let the supervisor take over" could itself block on
  the same wedge it was trying to escape. All three sites now call
  `std::process::exit(1)` directly, skipping the destroy call entirely --
  per design principle 4 (the mains switch IS the shutdown path), an abrupt
  exit here is not a shortcut, it is correct: the kernel reclaims the DRM
  master and every other resource on process exit regardless.
- **Wording fix (libmpv review, minor).** The recovery log line and
  `health.rs`'s module doc said "forces mpv to reconfigure the VO", which
  overstates what a v0.40 playlist replace actually does: it tears down and
  rebuilds demux+decode and forces a `vo_reconfig`, but does NOT tear down
  the video output itself (`uninit_video_out` only runs on process
  termination). Re-scoped in both places. Practical implication, NOT yet
  verified: tier 0 plausibly repairs a decode-side wedge but may not repair
  a fault in the DRM/GPU context itself (the HDMI-blink / late-waking-sink
  class the feature was motivated by) -- that may still need tier 1. Needs
  a physical HDMI-unplug bench test to settle; not done in this pass
  (device-level verification, not a code fix).
- **Not yet verified (libmpv review, SUSPECTED MINOR): startup grace for
  the `None` path.** `HealthMonitor` gives `Some` values one tick of
  "first sample" grace but `None` values none (deliberate -- see
  `health.rs`'s test doc). If a display takes longer than the ~20 s
  (2-tick) window to produce its first `time-pos` sample during a genuinely
  slow-but-not-failing cold start (a projector waking from standby against
  DRM modeset), F1 would issue a recovery mid-startup. Plausible slow paths
  either fail fast (already tier 1's job) or complete well under 10 s in
  practice, and 20 s to first sample has comfortable margin over measured
  4K decode startup (~1-3 s) behind `dex-wait-hdmi`'s EDID gate -- but this
  is inference, not a measurement of a genuinely slow cold VO bring-up.
  Worth one bench measurement before the next gallery install; not done in
  this pass.

All four Rust-level fixes verified: `cargo check --all-targets` and
`cargo test --lib` clean on the Mac (48 lib tests); full matrix re-verified
on the Pi (see the phasing note / session log for the exact count).

### F2 — Tier-1/2: supervision and reboot escalation
`deploy/dex-loop.service` exists (tier 1). Add tier 2: a `StartLimitBurst`
counter feeding an `OnFailure=` unit that reboots after repeated failures within
a window. Deliberately *not* the default `StartLimitAction=reboot`, because that
interacts badly with `StartLimitIntervalSec=0` (never-give-up restarts); needs an
explicit second unit. Future iteration per Max — record the design now.

### F3 — Bind fps (and mode) to the asset; refuse unbound assets
**Why:** the one failure that is undetectable *by construction*. A raw Annex-B
stream has no timestamps, so `--fps 25` on a 30 fps asset plays 20 % slow,
forever, with zero errors and every metric nominal.
**What:** ingest writes `loop.265.json` — `{fps, width, height, sha256, source,
encoder_cmd}`. `dex-loop loop.265` reads it, takes fps/mode from it, and refuses
to start if the sidecar is missing or the hash does not match the bytes it just
read (it already holds the whole payload; hashing is one pass). `--fps` survives
as a bench-only override.

### F4 — Asset validation gate at startup
**Why:** a truncated `.265` (interrupted copy) splices mid-NAL into the leading
IDR and glitches on *every* wrap — ~29k visible artefacts/day, silently.
**What:** scan the leading NAL units; require VPS(32)/SPS(33)/PPS(34) and an
IRAP slice (16–23) before any non-IRAP slice. Refuse otherwise. The F3 hash
covers truncation exactly; this covers *wrong-but-intact* assets (open GOP, no
leading IDR) that would break the gaplessness premise.

### F5 — Read-only rootfs + spare card
Not code. Dozens of mains cuts on a stock ext4 root is the most likely way the
installation dies permanently, and no `Restart=always` recovers a corrupt SD.
Overlay FS, swap off, a flashed spare taped to the plinth. Accept volatile logs.

### F6 — Baked EDID
Not code. `drm.edid_firmware=HDMI-A-1:edid/dex.bin` +
`video=HDMI-A-1:3840x2160@30D` in `cmdline.txt`, so boot order stops mattering
at the KMS layer rather than being worked around in userland.

### F7 — Nits
Version/git hash in the startup line; heartbeat log every ~10 min (loop count,
temperature, drop counters) so degradation is diagnosable after the fact;
`--drm-format` comment on the plane-swap portability caveat (done in code, keep).

**2026-08-15 addendum, from three further adversarial reviews:** F7's git hash was
always `nogit` on the ONE host that can actually build this (libmpv only exists on
the Pi, and the Pi builds from an rsync mirror at `~/bench/dex-loop`, which is not a
git checkout — confirmed live). `build.rs` now prefers a `.dex-build-id` stamp file
over `git rev-parse` when present (implemented). **Still outstanding:** the Mac-side
sync step that populates it does not exist yet as a script in this repo (today it is
presumably an ad-hoc `rsync` invocation) — before the next Pi build, that step needs
one line writing the source commit hash (`git rev-parse --short=12 HEAD`, `+dirty`
appended if the source checkout has local changes) to
`~/bench/dex-loop/.dex-build-id` before the rsync. Until that lands, the Pi binary
still reports `nogit`.

### F8 — On-site photographable failure signal (deferred — device config, needs on-device verification)
**Why (gallery-ops review, 2026-08-15):** every refusal this crate can produce — gate
(exit 2) or runtime (exit 1) — is journal-only. On tty1 there is no getty (conflicted
away so the player can take DRM) and stderr goes to journald, so on site every one of
these failure classes photographs identically: a black rectangle. This round of
hardening added four new gate classes (sidecar hash mismatch, sidecar missing, CRA
asset, rejected mpv option) without adding any on-site signal for any of them.
**What:** `deploy/dex-loop.service` could carry an `ExecStopPost=+/bin/sh -c '...'`
(privileged `+` prefix) that writes the last refusal line to `/dev/tty1` — fbcon owns
the display exactly when dex-loop has refused, so the reason becomes literally
photographable by gallery staff. **Why deferred rather than implemented now:**
fbcon/tty1 ownership and the exact escape sequence needed are hardware- and
kernel-version-dependent claims that need verifying on the actual Pi + projector, not
something to land blind in a code-review pass. A cheap partial mitigation (the
`journalctl -u dex-loop -n 20` triage line) was added to the README in the meantime.

### F9 — Heartbeat's property read is the first synchronous mpv-core call from the event thread
**Why (gallery-ops review, 2026-08-15, SUSPECTED):** `emit_heartbeat` calls
`mpv_get_property_string` from the event loop thread. If the mpv core is ever wedged
(e.g. the VO thread stuck in a DRM ioctl against a dying projector, holding the core
lock), this call could block — which would silence the heartbeat AND stop the event
loop from ever returning to `mpv_wait_event`, i.e. bug #1's exact shape (alive,
supervisor green, screen black) entered through a new door. Before this change the
event loop never touched the core at all. The reviewer could not force this scenario
without owning the display, hence SUSPECTED, not CONFIRMED.
**What (the reviewer's own suggested next step, worth taking seriously):** the
heartbeat plumbing makes a systemd watchdog nearly free to add — `WatchdogSec=` in
the unit plus a hand-rolled `sd_notify` (one UDP-style datagram to `$NOTIFY_SOCKET`,
std-only, no new dependency) turns the heartbeat from a diagnostic into a gate:
systemd kills and restarts the unit if the notify datagram stops arriving, covering
exactly this wedge. This is also the honest answer to F1's gap (periodic health
check) using infrastructure that already exists. Not implemented in this pass:
needs its own design + on-device soak, not a drive-by addition to an unrelated
hardening pass.

**2026-08-15 addendum, stakes sharpened by F1 (rust + gallery-ops reviews,
SUSPECTED MAJOR, still deferred):** F1 lands on top of this un-mitigated risk
rather than closing it. The heartbeat's two synchronous `mpv_get_property_string`
calls run on the SAME event thread, BEFORE the health-check tick in the loop
body, so if the ~600 s heartbeat boundary lands inside the stall-to-escalate
window (now roughly 2-3 ticks × 10 s ≈ 20-30 s per episode, before C1's fix;
was theorized to be somewhat longer with the pre-fix baseline-reset bug), a
core wedge coinciding with that boundary blocks `get_prop` and disables BOTH
tier 0 and tier 1 for that episode -- bug #1's exact shape (alive, green,
black wall), entered through the one door F1's own module doc claims is
closed "by construction". The claim is feature-locally true (F1 itself adds
no new synchronous call) but doesn't hold once F7's pre-existing heartbeat is
accounted for.

**2026-08-15, resolved.** The heartbeat no longer calls into the mpv core at all.
`mpv_get_property_string` and `mpv_free` are gone from the FFI surface entirely, and
`emit_heartbeat` no longer takes an `mpv_handle` — so the blocking read is not merely
avoided, it is unreachable. `frame-drop-count` and `vo-delayed-frame-count` are now
`mpv_observe_property(MPV_FORMAT_INT64)` subscriptions read out of
`MPV_EVENT_PROPERTY_CHANGE`, the same door F1 uses. Verified against mpv v0.40.0
source: both counters sit in `mp_event_property_change[MPV_EVENT_TICK]`
(`player/command.c:4484-4492`) beside `time-pos`; change events fire only on an
actual value change (`player/client.c:1715-1717`) so a clean run costs one event per
counter for the whole run; property-change events are never queued
(`player/client.c:942-943`) so they cannot contribute to the fatal `QUEUE_OVERFLOW`
path; and the getter runs on the core thread with the client lock dropped
(`player/client.c:1694-1699`), so a wedged core produces silence rather than a
blocked caller. The suspicion itself was upgraded to confirmed on the way:
`mpv_get_property_string` → `run_locked` → `mp_dispatch_lock`
(`misc/dispatch.c:364-394`) waits on a condition variable with **no timeout** until
the core thread is trapped in its dispatch loop. The line also gained
`pos=`/`pos-age=` — after removing the synchronous read, that freshness field is the
only honest liveness statement the heartbeat can make — and the counters now
accumulate across the per-file resets a tier-0 recovery causes, so `frame-drops=0`
can no longer be a false all-clear. **On-device verification still owed:** within
~1 minute of playback the counters must read numbers, not `n/a` — if they do not,
`MPV_FORMAT_INT64`'s transcription is wrong (it fails safe, but it fails).

**F10 — systemd `WatchdogSec=` + `sd_notify` (new, deferred).** Not a substitute for
the above and deliberately not bundled with it. It remains worth doing for the one
risk F9 does not touch: a hang in our own event-thread code that is not an mpv call
(`eprintln!` against a wedged journald). Constraints if it lands: keep `Type=simple`
and add `NotifyAccess=main` — never `Type=notify`, which can leave the unit inactive
forever if `READY=1` is not sent; `WatchdogSec` ≥ 180 s so it cannot preempt tier-0's
worst-case ~2-minute episode and turn every recoverable HDMI blink into a process
restart; ping `WATCHDOG=1` from the 10 s health-check tick, not the 600 s heartbeat.

### F3 addendum — sidecar JSON subset: deliberately NOT widened to arbitrary JSON types
**2026-08-15:** two reviews independently flagged that a non-subset *value* under
ANY key — including keys this player ignores entirely — refuses startup, and that
`\uXXXX` string escapes specifically would break on a default-safe JSON serializer's
output (Python's `json.dumps` `ensure_ascii=True`, Go's `encoding/json` escaping of
`<`/`>`/`&`) even inside an ignored key like `source`. **Fixed:** the string grammar
now supports `\uXXXX` (including UTF-16 surrogate pairs) everywhere, closing the
concrete, near-certain trigger. **Deliberately NOT done:** widening the *value*
grammar itself to accept arbitrary JSON types (booleans, `null`, floats, arrays,
nested objects) under keys this player doesn't interpret. Reasons: (1) the milestone-3
ingest tool that would actually emit such values does not exist yet — this is
speculative scope for a hand-rolled, gate-critical, zero-dependency parser; (2)
unbounded recursive value-skipping (nested arrays/objects) is itself new attack
surface in a parser whose entire job is "fail closed, predictably" — a
stack-depth concern that a fixed, flat, scalars-only grammar does not have. If the
ingest tool that eventually gets built needs richer informational metadata than a
flat string/uint, revisit then, with a concrete grammar to test against rather than
a hypothetical one. In the meantime this constraint is stated in both the module doc
and the README, not just implied.

### T note — `resolve_fps` string-equality is exact-match by design
**2026-08-15 (libmpv review, NIT):** `--fps 30` does not match a sidecar `"30/1"`
(exactly what an ffprobe-driven ingest would emit for `r_frame_rate`), by design —
the operator remedy is dropping `--fps` (the sidecar is authoritative regardless).
No code defect; recorded so the eventual ingest tool and any deploy scripts commit to
ONE canonical fps spelling rather than drifting.

### Bench note — sidecars needed before the next soak
**2026-08-15 (gallery-ops review):** the currently-running soak was started with the
pre-hardening binary and the bare `--fps 30 --mode ...` invocation; `~/bench` has no
`.json` sidecars. That exact invocation now exits 2 under the new binary (the
sidecar gate is working as intended) — generate `<asset>.json` sidecars for the bench
assets before the next soak launch (see the one-liner in this README's Options
section). The final multi-day soak (phasing step 8) should deliberately exercise the
sidecar path, not `--bench-no-sidecar`, since that is what actually ships. Separately:
the ingest procedure (once it exists) should end with a mandatory bench test-play —
`tests/cli.rs`'s `truncated_asset_vs_full_hash_refused_exit_2` fixture is a reminder
that an asset corrupted *before* ingest hashes "correctly" and can otherwise push mpv
into F1's unbounded-probe hole.

## T — Test coverage

Grounded in **bugs actually found**, not in coverage percentage. Every row below
maps to a defect that reached the bench or the review.

### T0 — Refactor for testability (prerequisite)
Extract the wrap arithmetic out of the FFI callback into a pure function:

```rust
/// Bytes to copy and the next position, given payload length and request size.
fn next_chunk(len: usize, pos: usize, want: usize) -> (usize, usize)
```

`read_fn` becomes a thin `unsafe` shell around it. This is the single change
that makes the core logic testable at all — today it is only reachable through
libmpv.

### T1 — Unit tests: the invariant that IS the program
| Test | Catches (real bug) |
|---|---|
| `next_chunk` never returns `n == 0` for any `want >= 1` | `read_fn` returning 0 = final EOF |
| `want == 0` handled explicitly and never conflated with EOF | same, the zero-length case |
| wrap at exact payload boundary; payload smaller than `want`; `want` far larger than payload | off-by-one at the wrap |
| `pos` always `< len` after any call | the position invariant the SAFETY comment asserts |
| **Concatenation property**: feeding N successive `next_chunk` calls reproduces the payload repeated endlessly, byte-for-byte, for arbitrary chunk sizes | the ONLY property that matters — that the stream really is the loop, repeated |
| 32-bit truncation: `want` derived from `u64::MAX` and from `2^32` saturates rather than becoming 0 | spurious EOF on 32-bit |

### T2 — FFI constant verification **against the live library**
The single highest-value test, because it catches the exact class of bug that
segfaulted this program: I invented `MPV_EVENT_LOG_MESSAGE = 6` (it is 2; 6 is
`START_FILE`), so the handler cast a start-file payload to a log-message struct.

mpv exposes the mapping at runtime — so assert it rather than trusting a
transcription:

```rust
// mpv_event_name(id) -> &str, and mpv_error_string(code) -> &str
assert_eq!(event_name(MPV_EVENT_LOG_MESSAGE), "log-message");
assert_eq!(event_name(MPV_EVENT_START_FILE),  "start-file");
assert_eq!(event_name(MPV_EVENT_END_FILE),    "end-file");
assert_eq!(event_name(MPV_EVENT_SHUTDOWN),    "shutdown");
assert!(error_string(MPV_ERROR_UNSUPPORTED).contains("unsupported"));
```

Requires declaring `mpv_event_name`. Runs against whatever libmpv is installed,
so it also catches a future mpv renumbering — which no header transcription can.

### T3 — Integration tests: the FAILURE paths
These did not exist, and every serious bug lived in them. Each asserts *exit
behaviour*, not output:

| Scenario | Expected |
|---|---|
| missing file | exit 2, message names the path |
| empty file | exit 2 |
| garbage bytes (undecodable) | **exits non-zero within N seconds** — never hangs |
| truncated valid stream | refused at startup once F4 lands |
| sidecar missing / hash mismatch | refused at startup once F3 lands |
| wrong `--fps` vs sidecar | refused |
| no display present | exits non-zero, does not idle-hang (needs a headless-capable assertion; may have to run on the Pi) |

The "never hangs" assertion is the one that would have caught the critical
review finding, and it is a timeout, not an output check.

### T4 — Tooling
* `cargo clippy -- -D warnings`, with `clippy::pedantic` advisory.
* `#![deny(unsafe_op_in_unsafe_fn)]` so every unsafe operation is explicitly
  scoped rather than inherited from the fn signature.
* **Miri** over the T1 unit tests (pure logic only; Miri cannot cross FFI) — the
  one tool that would flag a pointer-provenance mistake like the `user_data`
  finding if it were reachable from safe test code.
* `cargo test` in CI is not available on the Pi; run tests on the Mac for the
  pure logic, and the T2/T3 suites on the Pi where libmpv exists.

### T5 — Harness regression tests
The monitoring has now produced **three** of its own bugs: a watcher whose
condition could never fire; a liveness check for the wrong process name after
libmpv was embedded; and silently merged runs (below). Add a self-test to
`soak-monitor.sh`: run one sample against a deliberately-stopped player and
assert it reports `player=0` and a full 11-field row. **A monitor that has only
ever seen success is untested.**

### T7 — Recommended, not implemented: a live-fire test for F1's recovery command (rust review, 2026-08-15)
**Why:** C1 (see F1's 2026-08-15 addendum) shipped and reached the bench without
ever having been exercised against a real mpv instance -- the crate's own
justification ("would need real decode, forbidden by the vid=no rule") turned
out to be wrong: the reviewer's probe used `vo=null` with a real (tiny,
software-decoded) video track to exercise `loadfile ... replace` headlessly on
the Pi with no DRM and no display touched, and that is exactly what caught C1.
**What:** a hidden `--force-recovery-after-secs N` bench-only flag that
artificially triggers `HealthAction::AttemptRecovery` N seconds after start
during otherwise-healthy playback, plus a bench test asserting the process
*survives* it (keeps playing, does not exit) using a `vo=null` build. This
would have failed before the C1 fix and pins it going forward the same way
`tests/ffi_constants.rs` pins the event-id transcriptions. **Why not done in
this pass:** it is new test *infrastructure* (a new CLI flag + a new bench
harness invocation path), not a fix to an existing defect, and landing it
alongside four structural changes to the same event loop in one pass raises
the odds of introducing exactly the kind of untested new path this task
exists to close. The pure-logic regression tests added for C1
(`is_expected_recovery_stop`, 3 cases) narrow but do not eliminate the gap
this closes; T7 is the real fix for "the recovery path is undertested" and
should land as its own small, reviewed change.

### T6 — `run_id` column, and a header guard
**Observed 2026-08-15.** The soak was restarted against the same output file
when its duration was changed from 12 h to 3 h. The monitor appends and only
writes a header when the file is empty, so the second run's rows were appended
to the first run's — and `elapsed_s` **restarts at 0 mid-file**. Every row is
individually honest (`iso_time` is correct) while the series as a whole is not:
anything plotting elapsed sees time run backwards. Nothing warns.

Two defects, one cause — the file is append-only and carries no run identity:

1. **No run identity.** Add a `run_id` column, set once at monitor start
   (`date +%Y%m%dT%H%M%S`, or the PID — anything stable per invocation and
   sortable). Prefer a column over a file-per-run: one file is far easier to
   analyse across a campaign, and an explicit `run_id` makes the merge *visible*
   rather than silent, which is the actual failure here.
2. **No schema guard.** The header is written only when the file is empty, so a
   *schema change* appends differently-shaped rows to an old file with no
   complaint. This already happened once, when `held_at` took the column count
   from 10 to 11. On startup: if the file exists and is non-empty, compare its
   first line to the header this version writes; on mismatch, refuse and say so
   (or rotate to `<name>.1`). Never append a row whose shape does not match the
   header above it.

Both are a few lines, and both matter more for the **multi-day** soak than they
did here: a mid-run restart over days would be much harder to spot by eye than
one obvious reset in a six-row table.

**2026-08-15 addendum, harness fixes from the same three-review round:**

- **soak-monitor.sh's ssh telemetry leg was unbounded (gallery-ops review,
  MAJOR, fixed).** `-o ConnectTimeout=10` only bounds connection
  *establishment*; a session that connects and then goes quiet (network
  drop mid-read, or a hung remote `vcgencmd` firmware call) could block
  `read` forever, silencing every row for the rest of a multi-day soak with
  no warning -- the exact failure class `ece396b` had just fixed for the
  capture step, one line lower. Fixed with three layers: ssh-level
  `BatchMode=yes -o ServerAliveInterval=5 -o ServerAliveCountMax=2` (catches
  a connection that goes quiet), a remote-side `timeout 10` wrapping the
  actual `vcgencmd`/`pgrep` calls (catches a healthy-transport-but-hung
  remote command, which ServerAlive cannot see), and a local `timeout 20`
  around the whole ssh invocation when available (belt-and-braces; degrades
  gracefully via `command -v timeout` on a Mac without GNU coreutils).
  Manually verified against both the real Pi (clean row) and an unreachable
  host (bounded at ~10s, `? ? ?` fallback).
- **soak-monitor.sh's node parse step had no failure fallback (rust review,
  MINOR, fixed).** The ssh step already degraded to `"? ? ?"` on failure;
  the node step (same script, a few lines above) did not -- a node
  crash/OOM mid-soak under `set -e` would kill the whole monitor with no
  row and no message. Now falls back to an honest zeroed row plus a stderr
  warning, matching the ssh step's pattern.
- **soak-monitor.sh's capture-timeout kill path assumed one ffmpeg pid
  (rust + gallery-ops reviews, NIT, fixed).** `pgrep -P "$cap_pid"` can
  print more than one pid; the kill calls now iterate over all of them
  instead of passing a possibly-multi-line string as one `kill` argument.
  Works today either way (capture.mjs spawns exactly one direct child,
  verified), but a future capture.mjs refactor could have silently reopened
  the leak this was written to close.
- **test-soak-monitor.sh's CAP bumped 20 → 30 (rust review, NIT, fixed).**
  The new ssh bound above can legitimately take up to its own ~10-20s
  worst case; the old 20s outer bound left only a few seconds of headroom
  on a loaded Mac.
- **make-sidecar.sh: a wildly wrong explicit `--fps` now refuses instead of
  warning-and-proceeding (gallery-ops review, MINOR, fixed).** A typo'd
  `--fps 3` (for 30) against a stream ffprobe could read a trustworthy rate
  from used to print a warning and generate the sidecar anyway; the hash
  then binds the wrong rate, `--check` passes forever, and the player runs
  10x slow with every metric green -- F3's exact target failure, laundered
  through F3's own tooling. Now refuses (exit 2) unless `--force` is also
  given, per principle 5 (operator error must be impossible, not merely
  detectable). Two new regression tests added to `test-make-sidecar.sh`.
- **make-sidecar.sh: `--fps`/`--out` as the last token no longer crashes on
  an unbound variable (rust review, NIT, fixed).** Under `set -u`, a
  trailing `--fps` with no value dereferenced an unset `$2` and aborted
  with exit 1 ("verification failure") instead of the correct exit 2
  ("bad invocation") via the normal `usage` path. Guarded with
  `[ $# -ge 2 ] || usage`. One new regression test.

## Phasing

1. **T0 + T1 + T2** — refactor and the tests that pin the core invariant and the
   FFI constants. Cheap, and closes the two classes that actually bit.
2. **F3 + F4** — asset binding and validation. Removes the undetectable failure.
3. **T3** — failure-path integration tests, including "never hangs".
4. **F1** — tier-0 self-healing. Design carefully against principle 1.
5. **T5 + T6** — harness: monitor self-test, `run_id`, header guard. Cheap, and
   the multi-day soak depends on the data it produces being trustworthy.
6. **F5 + F6** — the device-level work (read-only root, baked EDID).
7. **F2 tier 2** — reboot escalation. Future iteration.
8. **Multi-day soak on the final build**, with the final asset, on the show
   hardware. Not before: soaking code that is about to change measures the wrong
   artifact.
