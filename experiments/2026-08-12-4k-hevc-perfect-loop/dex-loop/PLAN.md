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

**2026-08-15, second addendum -- overlapping tier-0 recoveries (MAJOR, confirmed by
three further adversarial reviews):** the C1 fix above absorbs exactly one
`END_FILE(reason=stop)` via a single `bool`. It missed that the organic
health-check tick and T7's forced probe run in the SAME loop iteration
(tick block before trigger block) and `mpv_command_async` only QUEUES a
`loadfile` against a core that may still be busy from a still-wedged
episode -- so a SECOND recovery can be issued (and accepted) before the
first attempt's stop has even been observed, e.g. an organic tick issuing
attempt 1 while the core is still wedged, then T7's forced probe issuing
attempt 2 in the same iteration, or (organic-only) two re-arms 10 s/20 s
apart against a core that unwedges only after both are queued. Two
in-flight recoveries produce TWO `END_FILE(reason=stop)` events; a bool
absorbs only the first and takes the fatal path on the second -- a healthy
recovery converted into an unnecessary tier-1 restart with a journal line
indistinguishable from C1. **Fixed:** `recovery_stop_pending: bool` ->
`recovery_stops_pending: u32`, incremented on every successfully-queued
recovery command and decremented on every absorbed stop (or a rejected
command reply); `is_expected_recovery_stop` now checks `count > 0`. Two new
pure-logic tests in `main.rs` pin the two-in-flight sequence and the
rejected-reply decrement. No suppression of the second issue was added
(considered and rejected as unnecessary complexity): the counter alone
makes both stops absorb cleanly, and spending budget slightly faster in
this edge case is consistent with the module's existing "lean on tier 1
sooner than strictly necessary" philosophy (see "why the budget never
resets" above).

**2026-08-15, third addendum -- smaller findings from the same review pass, all
confirmed against mpv v0.40.0 source (`/tmp/mpv-0.40`) and fixed (comment/doc-only
or cheap logic, per this task's MINOR/NIT bar):**
- `read_fn`'s SAFETY comment attributed callback exclusivity to a
  serialization guarantee `stream_cb.h` does not make (it makes none;
  `cancel_fn` is explicitly documented cross-thread). Reworded to attribute
  it to mpv's stream layer instead (single-owner `stream_t`, open-time probe
  happens-before reads, close happens-after teardown) and to warn that
  wiring up `cancel_fn` (still `None` today) would break this argument.
- The comment justifying registering F9's two counters before `loadfile`
  claimed the forced initial notification "arrives as soon as the VO chain
  exists" and would otherwise be "missed by a later subscription" -- both
  wrong per `client.h`/`player/client.c` (the initial event is forced
  unconditionally at registration and arrives promptly as
  `format=NONE`/`data=NULL`; nothing is ever missed by registering late).
  Reworded to the accurate rationale (registration order doesn't affect
  completeness; it is grouped here only for tidiness).
- `heartbeat.rs`'s module doc claimed a clean run costs "one event per
  counter" -- actually at least two (the forced initial `NONE`, then a
  separate event when the VO chain first makes the property available).
  Reworded; the "negligible, never queued" cost argument itself was correct
  and stands.
- `ObservedCounter` could under-count drops across a recovery: mpv coalesces
  property-change events (`client.h`: "only once the event queue becomes
  empty ... one event per changed property"), so if a recovery's teardown
  (unavailable), the new session's restart at 0, and a climb past the old
  total all happen before this program's event thread next drains, `sample`
  never sees the intervening decrease and under-counts. Fixed: added
  `ObservedCounter::mark_unavailable()`, called from `main.rs`'s
  property-change handler on `MPV_FORMAT_NONE` for the two drop-counter
  userdata tags (previously silently ignored), which clears the diffing
  baseline without un-earning an already-known total (needed a new
  `has_sample` field, separate from the baseline, so the total doesn't
  flicker back to "n/a" for the same few-second gap). New
  `MPV_FORMAT_NONE` constant in `ffi_consts.rs`. Narrows the under-count
  window rather than closing it (the `NONE` event itself could still be
  coalesced away); full closure isn't possible from the client side and
  isn't worth more machinery for a diagnostic line.
- The `n/a` doc comment attributed persistent `n/a` only to "the VO chain
  never came up", but `mpv_observe_property` never validates a property
  NAME (`client.h`: "Observing a property that doesn't exist is allowed") --
  a future mpv rename of `frame-drop-count`/`vo-delayed-frame-count` would
  subscribe successfully and sit at `n/a` forever, not fail loudly at
  `observe()`. Doc comment extended to name that second cause.
- T7's arming warning and `--force-recovery-after-secs`'s usage text said
  the probe fires "N seconds into playback" / "after the stream starts" --
  it actually fires N seconds after the `loadfile` request is queued
  (`started = Instant::now()` right after that call), which is not the same
  moment on a real asset with non-trivial decode startup latency. Reworded
  both strings to say what they mean, and added a note to size N with
  margin over real decode startup for a live-fire run against a real asset.
  The deeper fix some reviewers suggested -- arm the trigger's clock from
  the first observed `time-pos` sample instead -- was NOT done: it is a
  small design change to `ForceRecoveryTrigger`/the driver, not a wording
  fix, and the one ignored test that exists today (`tests/cli.rs`'s
  `force_recovery_survives_against_real_mpv`) uses a synthetic stub asset
  with millisecond decode startup, so the gap has zero practical effect on
  it as written. Revisit if a live-fire procedure against a real 4K asset
  is ever added.
- Verification note, no defect: the same review pass confirmed (1) no F9
  heartbeat field became silently less meaningful and no wedged-core state
  can present as healthy, and (2) T7's `--force-recovery-after-secs` cannot
  reach a real deployment (CLI gate, refused before the asset is even read,
  pinned by tests; `deploy/dex-loop.service` passes neither bench flag).
  No action.

All Rust-level fixes across both addenda verified: `cargo check --all-targets`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test --lib` clean on the
Mac (70 lib tests). **On-device verification still owed** for all of it -- the Pi
was off-limits for this pass (mid-thermal-soak); nothing here has been exercised
against a real mpv since these changes landed.

### F2 — Tier-1/2: supervision and reboot escalation
`deploy/dex-loop.service` exists (tier 1). Add tier 2: a `StartLimitBurst`
counter feeding an `OnFailure=` unit that reboots after repeated failures within
a window. Deliberately *not* the default `StartLimitAction=reboot`, because that
interacts badly with `StartLimitIntervalSec=0` (never-give-up restarts); needs an
explicit second unit. Future iteration per Max — record the design now.

**F10 note (2026-08-17):** the unit now also carries `WatchdogSec=180` (F10) —
a watchdog-triggered kill is `Restart=`-recovered exactly like an exit-code
failure (confirmed live on the Pi: `Result: watchdog`, `Killing process ...
with signal SIGABRT`, followed by the same `Scheduled restart job` path an
ordinary crash takes), so watchdog expiries are already IN-BAND with tier 1 and
need no special-casing here. When tier 2's `StartLimitBurst`/`OnFailure=`
counter is eventually built, a watchdog-caused restart should count toward it
the same as any other — nothing in F10 requires tier 2 to distinguish the two.

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

### F5 — Read-only rootfs + spare card — CLOSED, not needed (Max, 2026-08-17)

Not code, and not a new risk. **Already proven in production runs**: existing dex
installations have survived mains-cut cycling on the current image arrangement,
and neither the new player nor the new OS base changes that exposure. Closing
rather than re-litigating a question the field has already answered.

The original concern is preserved for context: dozens of mains cuts on a stock
ext4 root is the most likely way an installation dies permanently, and no
`Restart=always` recovers a corrupt SD. Overlay FS, swap off, a flashed spare
taped to the plinth.

**One caveat worth re-checking rather than assuming, if it ever bites.** The
production evidence comes from *dexOS* images. The M5 bench card is plain Debian
trixie + this `.deb`, which does **not** inherit dexOS's arrangement. If the
shipping card stays plain-trixie rather than becoming a trixie dexOS image, the
"already proven" argument is about a different filesystem setup than the one
deployed. Not a reason to reopen F5 now; a reason to notice if a card ever comes
back corrupt.

### F6 — Display mode belongs to the exhibit config (Max, 2026-08-17: DO IT)

> **DESIGN SETTLED BY MAX, 2026-08-17. Read this before implementing — it
> supersedes any plan that puts display config in the asset sidecar.**
>
> **An exhibit file: `dex.yaml`, which REFERENCES the asset.** Not the sidecar.
>
> - The exhibit file names *which* asset to play, so **several assets can sit in
>   storage** and the exhibit picks one. That is the feature the sidecar cannot
>   provide at all, and it matters more than where the mode lives.
> - **YAML, not JSON**, because a human edits this in the field, possibly on a
>   phone over SSH. JSON is a subset of YAML, so a machine can still emit plain
>   JSON and it parses — we get human-editability without giving up
>   machine-writability.
> - The sidecar keeps its current job unchanged: fps + sha256 bound to the asset
>   *bytes*. That is asset integrity. The exhibit file is installation
>   configuration. Two different lifetimes, two different files.
>
> **A reasoning error of mine, corrected, because it could mislead an
> implementer.** I argued the sidecar was the wrong home because it is
> "hash-bound, so changing the display would require re-hashing the artwork."
> That is backwards. The hash covers the asset's own bytes; editing a `mode:`
> field beside it would not invalidate anything about the asset. The sidecar is
> the wrong home for a *better* reason: **an exhibit is not a property of an
> asset**. One artwork may run on several panels, and one panel may show several
> artworks over a season. Config whose lifetime differs from the thing it sits
> next to will eventually be edited in the wrong copy.
>
> **REFINED BY MAX, 2026-08-17 — both formats, honestly.** Support **`.json`
> AND `.yaml`**, and **the extension decides the parser**: a `.json` file must be
> strict JSON, a `.yaml` file may use YAML. Share as much code as possible
> between the two paths.
>
> **Do not take the tempting shortcut.** YAML is a superset of JSON, so "parse
> everything with the YAML parser" would pass every test and be wrong: it would
> silently accept comments, anchors and unquoted keys inside a file named
> `.json`. That file would then break every *other* tool that reads it as JSON —
> `jq`, `python -m json.tool`, any future web UI. **The extension is a promise to
> the rest of the world about what the bytes are**, and honouring it is the whole
> point of offering two formats rather than one.
>
> **Shape that maximises sharing:** each parser produces a generic tree
> (`serde_json::Value` / `yaml_rust2::Yaml`); one shared function maps that tree
> to the `Exhibit` struct and validates it. Only the tree-building step differs,
> which is a handful of lines — everything semantic is common.
>
> **Dependency: `yaml-rust2`.** Measured 2026-08-17 against the §5c policy:
>
> | crate | crates added | proc macros |
> |---|---|---|
> | **yaml-rust2** | **8** | **0** |
> | serde_yaml_ng | 10 | 0 |
> | saphyr | 20 | **6** — would fail deny.toml's ban outright |
>
> `serde_yaml` itself is archived (2024) and is not an option.
>
> **Also still open on F6 (see below):** the exhibit file must NAME ITS ASSET, so
> several assets can sit in storage and the exhibit picks one. That was the
> capability Max asked for and the first implementation does not have it —
> `ExecStart` still hardcodes `/opt/dex/loop.265`.
>
> **And a factual error to fix in `src/exhibit.rs`'s module doc:** it cites "the
> M5 soak played a 2160p30 asset on a 1440p Dell". That did not happen — the soak
> ran 4K30 on the Cam Link, with the Dell disconnected. The design conclusion is
> right; the evidence cited for it is invented, which is worse than citing none.

> **Decision the implementer still owns:** YAML needs a parser, and SPEC §5c says
> a dependency earns its place by removing code we would otherwise own. Note
> `serde_yaml` is unmaintained (archived 2024); `serde_yaml_ng` and `yaml-rust2`
> are the live options. Weigh them, state the choice, and remember a new runtime
> dependency also changes the `.deb`'s derived `Depends`. Since JSON is valid
> YAML, "reuse serde_json and require JSON syntax" is a legitimate third option —
> but it forfeits comments, which is much of why a human wants YAML.

**Decision: the target display is part of the "exhibit" configuration.** An
explicit mode is the default because an exhibition wants stability, and `auto`
stays available for flexibility (bench work, an unknown venue panel, a swap
mid-install).

Not code at the KMS layer: `drm.edid_firmware=HDMI-A-1:edid/dex.bin` +
`video=HDMI-A-1:3840x2160@30D` in `cmdline.txt`, so boot order stops mattering
at the KMS layer rather than being worked around in userland.

**Why this is not merely boot-order insurance — measured 2026-08-15/16, in both
directions on one bench:**

- The **Cam Link 4K** advertises `3840x2160@30` as its *preferred detailed
  timing* (plus CTA VIC 95/94/93 and HDMI VIC 1/2), and `vc4` builds **zero**
  3840x2160 modes from it. Forced, the identical 297 MHz timing works. So a
  correct EDID is not sufficient — the driver declines a mode the sink asks for.
- The **Dell U2719DC** is 2560x1440 and its EDID never mentions 2160. It must
  **not** carry the force, or the Pi transmits a signal the panel cannot show
  and it reads as a player fault.

So the same `cmdline.txt` line is correct for one sink and wrong for the other,
and "remove it as stale" is a change of target rather than cleanup — a mistake
made and reverted within four hours on 2026-08-15. Hence: **per-exhibit config,
stated explicitly, not negotiated.**

Current stopgap, which this replaces: a systemd drop-in overriding `--mode`,
plus a hand-edited `cmdline.txt`, plus a comment block in `config.txt` carrying
the reasoning. Three places, none of them a config file.

**2026-08-17, implemented — and a decision conflict surfaced, not resolved
here.** Landed as `src/exhibit.rs` (grammar + `ExhibitConfig` + `resolve_display`
+ the cmdline comparator + the sysfs pre-flight parser + `reconcile_cmdline`,
37 unit tests, all Mac-testable) plus `main.rs` wiring (new startup gates,
before the asset is even read: config parse → cmdline-vs-kernel gate → sysfs
mode pre-flight) plus a new privileged sibling binary,
`src/bin/dex-exhibit-apply.rs`, that reconciles `cmdline.txt` and is the only
thing in this crate that writes boot config. `deploy/dex-loop.service` no
longer passes `--mode`. Full detail in README.md's "Exhibit config (F6)"
section and this task's session notes.

**The conflict:** this implementation is **JSON**, at **`/etc/dex/exhibit.json`**,
and covers **display config only** — it does NOT let the exhibit file select
*which asset* plays (the asset path is still the one baked into
`dex-loop.service`'s `ExecStart`, `/opt/dex/loop.265`). That contradicts the
"DESIGN SETTLED BY MAX, 2026-08-17" blockquote directly above, which mandates
**YAML**, a file named **`dex.yaml`**, and explicitly requires asset
*selection* ("several assets can sit in storage and the exhibit picks one")
as the primary reason JSON/the sidecar's parser was rejected there.

This implementation instead followed a SEPARATE, more detailed design brief
prepared for this specific task (fresh bench evidence gathered the same day:
the `modetest` mode-list discrepancy, the `override.conf` residue, the stale
`config.txt` comment — see the story file) that settles on JSON + the
sidecar's existing subset parser + display-only scope, and does not mention
or explicitly supersede the YAML/`dex.yaml`/asset-selection blockquote above.
Both are labeled as Max's own settled decisions; they were not reconciled
before this implementation started, and this implementer followed the later,
more concrete brief per this task's own instructions rather than silently
picking a side. **Two things Max should resolve:**

1. Confirm whether JSON `/etc/dex/exhibit.json` (display-only) is the
   INTENDED supersession of the YAML `dex.yaml` blockquote, or whether that
   blockquote's asset-selection requirement is a still-open, separate need —
   in which case it is a different feature (WHICH asset plays) from what F6
   as shipped covers (WHAT MODE the display uses), and probably deserves its
   own F-item rather than retrofitting into this one.
2. If asset selection is still wanted, decide whether it belongs in
   `/etc/dex/exhibit.json` (widening this format, and probably requiring a
   YAML migration at that point for the comment-support reason the blockquote
   gives) or in a still-separate file — rather than resolved by drift.

**2026-08-17, adversarial-review pass (two reviews of F6/F10; fixes applied on
this branch).** Fixed here:

* **`dex-exhibit-apply` now writes `cmdline.txt` durably and atomically** —
  the old `fs::write` was open(O_TRUNC)+write with no fsync: a mains cut
  mid-apply (or any time before the page cache flushed — and the tool's next
  printed word invites a power cycle) could leave a zero-length/garbage
  `cmdline.txt`, an unbootable Pi recoverable only by pulling the SD card on
  site. Now: temp file + `sync_all` + `rename` + directory sync; the backup
  is also synced BEFORE the original is touched, backup names get a counter
  suffix on same-second collision, and only the 5 newest backups are kept
  (the boot FAT32 partition is small).
* **EACCES no longer masquerades as "file missing"** — an unreadable-but-
  present `/etc/dex/exhibit.json` (0600 root edit, wrong-owner restore) used
  to produce "Create /etc/dex/exhibit.json" for a file the operator can see
  exists. Now only `NotFound` defers to that message; anything else exits 2
  naming the real error and the chmod/chown repair.
* **The cmdline gate names BOTH repairs** — its old message prescribed
  `dex-exhibit-apply` unconditionally, but in the stale-CONFIG direction
  (fresh default config installed over a cmdline whose force the venue
  needs — the Cam Link case, this file's own example) following that
  instruction deletes the needed force and `auto` then hides the downgrade.
* **Connectorless `video=WxH@R` tokens (no `conn:` prefix) are refused, not
  invisible** — the kernel grammar accepts them and they force ALL
  connectors; both the gate and `reconcile_cmdline` previously could not see
  one (gate reported "no token"; the reconciler would append a second,
  overlapping force). Now both refuse, naming the token.
* **`auto` + sidecar resolution warning** — `display_mode: "auto"` skips the
  sysfs pre-flight by construction, which converts fail-closed into
  fail-silent on the exact config the `.deb` ships: a forgotten exhibit.json
  on hardware that builds no 4K mode unforced plays the 4K artwork at the
  connector's fallback for the run of the show, every metric green. The
  sidecar is hash-bound and already carries width/height, so `main.rs` now
  WARNS (F8-style, at startup) when `auto` is in effect and the connector's
  mode list does not offer the asset's resolution.
* README + `man dex-exhibit-apply` no longer overclaim the pre-flight: it
  validates the RESOLUTION half of `display_mode` only. The `@R` refresh half
  is grammar-checked and settled by mpv's `--drm-mode` at VO init (the
  kernel's sysfs `modes` file has no refresh column — verified on dexpi4, six
  indistinguishable `3840x2160` lines).
* **Refresh behavior bench-verified on dexpi4** (2026-08-17, real deploy path,
  installed .deb, Cam Link forced to 4K30), answering the review's open
  question three ways:
  1. `display_mode: 3840x2160@60` (integer, not offered): mpv REFUSES at VO
     init — `mpv/vo/gpu/drm: Could not find mode matching 3840x2160@60` →
     exit 1 → 2 s restart loop. Loud, not silent — but mpv-voiced, not
     gate-voiced.
  2. `display_mode: 3840x2160@30000/1001` (rational): mpv's OPTION PARSER
     rejects it — `set drm-mode=...@30000/1001: error setting option (-7)` →
     exit 2 loop. A value the old grammar accepted could NEVER play.
  3. `display_mode: 3840x2160@29.97` (decimal): parses and PLAYS — mpv
     matches DRM modes by integer vrefresh rounding, i.e. it silently drove
     the same 30 Hz mode `@30` names honestly.
  Consequence, fixed here: `display_mode`'s refresh grammar is now INTEGER
  ONLY (same rule as `kms_force`), refusing at config parse the rational
  form that could never play and the decimal form that only pretended to
  mean something. The old doc claim "mpv's `--drm-mode` accepts a fractional
  refresh" was written from mpv's manual, never driven, and is false in
  practice.
* usage() no longer calls `--mode` "REQUIRED alongside --bench-no-sidecar"
  while stating its default (`bench_with_no_mode_defaults_to_auto` pins the
  actual behavior: optional, defaults to auto).
* Test count: `src/exhibit.rs` now has 40 `#[test]` fns (37 at 6df57ed — the
  number this entry used to state, which was CORRECT; the commit message's
  "38" is off by one, and a review claim of "27" is wrong. The commit message
  also carries a Co-Authored-By trailer against this repo's convention — both
  are pushed history, noted here rather than rewritten).

Deferred, with reasons:

* **Refuse (vs warn) on the `auto`/asset-resolution mismatch, and/or shipping
  `display_mode` as an explicit `"unset"` that refuses like a missing
  config** — a real behavior change to the shipped default, entangled with
  the parked JSON-vs-`dex.yaml` design questions above. Max's call.
* **Refresh-half pre-flight extension** (mpv mode enumeration, or `modetest`
  parsing at apply time) — the bench data above settles what it would buy: an
  unoffered INTEGER refresh already fails loudly at VO init (restart loop
  with mpv's error in the journal), so the extension would only upgrade the
  error's voice from mpv's to the gate's operator-grade message. Worth doing
  eventually, not load-bearing; belongs with whichever F6 design Max lands.
* **postinst warning when the fresh default conffile is installed over a
  cmdline that already carries a `video=` token** — the predictable upgrade
  collision. Packaging design (conffile semantics, upgrade vs fresh install
  detection); belongs with whichever F6 design Max lands.

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

### F8 — On-site photographable failure signal (Max, 2026-08-17: YES, but POST-1.0)
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

### F10 — systemd `WatchdogSec=` + hand-rolled `sd_notify` (2026-08-17, IMPLEMENTED)

**Why:** not a substitute for F9/F1 and deliberately not bundled with either. It
closes the one risk neither touches: a hang in our OWN event-thread code that is
not an mpv call at all — canonically `eprintln!` blocking against a wedged
journald, including on the `Escalate` arm whose entire job is "exit so tier 1 can
take over." Nothing in-process can detect that; it needs an external actor.

**Liveness criterion (the requirement that it be one a WEDGED PLAYER FAILS):**
`main.rs` pings `WATCHDOG=1` once per `HEALTH_CHECK_SECS` (10 s) tick, AFTER
that tick's `HealthMonitor::tick` evaluation and any resulting recovery command
have completed — never from the 600 s heartbeat. F1's recovery budget
(`MAX_RECOVERY_ATTEMPTS`) is cumulative and never refills, so a display-wedged
player's tick sequence is FORCED, by construction, through silence → stall (2
ticks) → ≤3 budgeted recoveries (~20 s each) → `Escalate` → `exit(1)`. A wedge
therefore emits a BOUNDED number of pings and then either exits (tier 1's
exit-code path already covers that) or, on the one path that can still hang
(e.g. the `eprintln!` before `Escalate`'s own `exit(1)`), simply stops pinging —
which is exactly what the watchdog is here to catch. Within the
wedged-core/wedged-thread hazard class there is no third state (but see
"residual states", below — the claim is deliberately NOT broader than that).
The ping deliberately sits OUTSIDE the `if let Some(h) = health` gate: if the
`time-pos` subscription itself failed to register (F1 disabled, near-zero
probability), stopping pings there too would convert a merely-degraded-but-alive
run into a guaranteed watchdog kill loop.

**No new blocking call:** `sd_notify` is `sendto(2)` on an `AF_UNIX SOCK_DGRAM`,
which — unlike UDP — has flow control and CAN block if the receiver's queue is
full. The socket this crate opens is explicit non-blocking
(`UnixDatagram::set_nonblocking(true)`), and `EAGAIN`/`EWOULDBLOCK` (or any other
send failure) is a DROPPED ping — incremented in a counter, never retried
synchronously, never panicked on. Surfaced in the heartbeat line
(`watchdog=armed pings-dropped=N` / `watchdog=inert`), so a degraded run is
diagnosable after the fact.

**Dependency: zero, hand-written, `std` only.** `std::os::unix::net::UnixDatagram`
covers the whole protocol, including the abstract-namespace case
(`std::os::linux::net::SocketAddrExt`, stable since 1.70, inside this crate's
`rust-version = "1.85"` floor). No `libsystemd` binding (would grow the `.deb`'s
`$auto`-derived `Depends` with a new shared-object link to save ~150 lines) and no
`sd-notify` crate (removes less than it appears to — the ping policy, the env
handshake, and an audit of its socket handling for the non-blocking guarantee
this feature requires would all still be owned here). `Cargo.toml`'s dependency
list and the `.deb`'s `$auto`-derived `Depends` are BOTH unchanged by this
feature — verified: `cargo tree` shows no new crate, and CI's own "Verify derived
dependencies" step (unchanged) still only asserts `libmpv`/`libc`.

**`WatchdogSec=180`, `NotifyAccess=main`, `Type=simple`** (never `Type=notify` —
a notify unit that never sends `READY=1` sits inactive forever, and this player
has no natural "ready" moment; `READY=1`/`STOPPING=1` are deliberately never
sent, see `src/watchdog.rs`'s module doc). 180 s is ≥17× the 10 s ping cadence
(tolerates a burst of dropped/missed pings) and comfortably exceeds F1's own
worst-case tier-0 episode (~2 min), so a watchdog kill can never preempt a
recovery tier 0 would have completed on its own.

**Bench probe (`--bench-wedge-after-secs N`):** mirrors T7's shape exactly —
requires `--bench-no-sidecar` (refused otherwise, gate-tested in `tests/cli.rs`),
absent from `deploy/dex-loop.service`. Deliberately parks the event thread in a
`loop { thread::sleep(...) }` forever once armed, simulating F10's exact hazard
class. Checked every loop iteration, before the event-id dispatch — with `N=0`
it fires on the very first iteration, unconditionally, before that same
iteration's dispatch can race it to an ordinary `exit(1)` (relevant under this
file's mandatory `vid=no --opt aid=no` test convention, where mpv reaches
"nothing to play" fast).

**Verification — Mac (`cargo check --all-targets`, `cargo clippy --all-targets --
-D warnings`, `cargo test --lib`):** all clean, 91 lib tests including 19 new
`watchdog::tests` (env-handshake resolution — no socket, no `$WATCHDOG_PID`/
`$WATCHDOG_USEC` edge cases; the exact wire payload; `interpret_send_result`'s
Ok/WouldBlock/other-error → Sent/Dropped mapping, pure; `socket_addr`'s
path/abstract construction, cfg-gated for non-Linux; and two REAL-SOCKET tests —
`send_ping` delivers the exact payload to a bound receiver, and a flood test
that proves the actual non-blocking property against an unread receiver's
UNMODIFIED default `SO_RCVBUF`, bounded at 200k attempts — see `src/watchdog.rs`'s
module doc for why this deviates from shrinking `SO_RCVBUF` via `setsockopt`: the
crate is `#![forbid(unsafe_code)]`, which cannot be locally overridden even in a
test). `heartbeat.rs` gained 2 new tests for the `watchdog=armed pings-dropped=N`
/ `watchdog=inert` rendering.

**Verification — Pi (dexpi4, tmux, 2026-08-17):** `cargo clippy --all-targets --
-D warnings` and `cargo test --release` both clean (91 lib + 5 ffi_constants + 27
cli passed, 1 pre-existing `#[ignore]`d test skipped as expected) — including the
new `socket_addr_resolves_an_abstract_target_on_linux` test (the one path the Mac
cannot exercise) and 4 new `tests/cli.rs` gate/mechanism tests for
`--bench-wedge-after-secs` (missing-value, non-numeric, requires-bench-no-sidecar,
and `bench_wedge_flag_actually_hangs_the_event_thread_forever` — proves the probe
genuinely, permanently parks the thread; the harness's own deadline-kill is the
only way that test ends).

Then the watchdog itself, live, via three throwaway `systemd-run` transient
units (unprivileged `User=dex`, matching production, `WatchdogSec` shortened for
a fast bench cycle — real values would take the full 180 s):

1. **`WatchdogSec=15` + `--bench-wedge-after-secs 0`, twice in a row:**
   ```
   f10-wedge-test.service: Watchdog timeout (limit 15s)!
   f10-wedge-test.service: Killing process 52281 (dex-loop) with signal SIGABRT.
   f10-wedge-test.service: Main process exited, code=killed, status=6/ABRT
   ```
   — fired again identically on the auto-restart (`Restart=on-failure`), 18 s
   later. **The watchdog fires and tier 1 recovers — the two-cycle reproduction
   this task explicitly demanded ("a watchdog never seen to fire is
   indistinguishable from one wired to nothing").**
2. **`WatchdogSec=8`, `ExecStartPre=/bin/sleep 15`, `ExecStart=/bin/sleep 30`
   (no pinging at all):** `Starting...` 14:50:21 → `Started...` 14:50:36 (exactly
   the 15 s `ExecStartPre` sleep) → `Watchdog timeout (limit 8s)!` 14:50:44 —
   exactly 8 s after `Started`, NOT 8 s after `Starting` (which would have fired
   at 14:50:29). **Confirms `ExecStartPre` consumes none of the watchdog
   budget** — previously stated as inference from systemd semantics; now
   measured.
3. **`WatchdogSec=15`, healthy run, real software HEVC decode (`--opt vo=null
   --opt aid=no`, vid enabled), sampled every ~8 s for 40 s+:** `WatchdogTimestamp`
   advanced every sample (~10-11 s apart, matching the ping cadence),
   `NRestarts=0` throughout, journal showed the `watchdog=armed pings-dropped=0`
   heartbeat line and zero further watchdog-related lines. **Confirms real pings
   register with PID 1 under `NotifyAccess=main` + an unprivileged `User=dex`,
   and a healthy run is never falsely killed.**

**Interaction with F1 (tier 0):** by construction, not by suppression — pings
fire on every completed tick regardless of `HealthAction` ∈ {Healthy,
AttemptRecovery}, so a mid-recovery player is still completing duty cycles,
still pinging; there is no window where "F1 mid-recovery" and "watchdog counting
down" can race, because the same loop iteration that advances F1's state also
feeds the watchdog.

**`.deb` impact: none.** No new crate, no new soname, `$auto`'s derived `Depends`
unchanged. The only shipped change is `deploy/dex-loop.service` gaining
`WatchdogSec=180`/`NotifyAccess=main`.

**Residual states (2026-08-17, from adversarial review — recorded, not bugs):**
two states satisfy the watchdog while the wall is black, and the liveness claim
above is scoped to exclude them on purpose:

1. **Health=None degraded mode** (the `time-pos` subscription failed at
   startup): the ping deliberately certifies only "the event loop iterates" —
   a subsequently display-wedged player pings forever. Documented and argued
   above (the alternative is a guaranteed kill loop); the cost is that the
   criterion's strength silently depends on an `mpv_observe_property` return
   code from weeks earlier, with the startup warning as the only trace.
2. **Signal-level failure** — HDMI signal lost mid-run, panel powered off,
   plane presenting to a disconnected sink: `time-pos` keeps advancing, F1
   reads Healthy, pings continue forever, wall stays black. No in-process
   criterion can see this. **Explicitly out of scope** for F10. Possible
   future closure, cheap if ever wanted: poll DRM connector `status` from the
   health tick — the same sysfs files F6's pre-flight and `dex-wait-hdmi`
   already read.

**Caveat on evidence item 3 (same review):** the "healthy run" proof used
`--opt vo=null` on a 1.3 MB 1080p card — the HEVC decode was real but the
DRM/KMS output path was NOT exercised under an armed watchdog (the 25.5 h soak
conversely ran the full VO path with no watchdog; that binary predates F10).
"Watchdog armed + real KMS output" belongs in the next long soak — which must
also be the first soak of F6+F10 TOGETHER: as of 2026-08-17 no build anywhere
contains both (F6's working implementation is parked on
`experiment/4k-hevc-perfect-loop-f6-json-poc`, branched before F10, and the
`.deb` on dexpi4 is that F6 build, watchdog-less; both lineages edit the same
`[Service]` region of `deploy/dex-loop.service`, so the merge must be done
attentively — a careless one could drop `WatchdogSec=180` or the
exhibit-config comments).

**Setup-failure policy (2026-08-17, post-review fix):** every
"`resolve()` said Armed but the ping socket cannot be established" arm used to
log "watchdog DISABLED for this run" and limp on — but nothing in-process can
disarm systemd's timer, so under the shipped unit that run would be
SIGABRT-killed every 180 s forever while the journal claimed the watchdog was
off. Now (`main.rs::watchdog_setup_failed`): if `$WATCHDOG_USEC` is present
(kill timer demonstrably armed) the process exits(1) for a clean
`RestartSec=2` retry — the failure class is transient, and a fast retry
strictly beats a `WatchdogSec`-cadence kill loop with a lying journal line;
if no timer is armed, it runs without pings as before (accurately worded).
The `WatchdogPidMismatch` inert arm additionally warns, when `$WATCHDOG_USEC`
is set, that systemd will kill the process every window — so the journal
explains the deaths that follow.

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

**2026-08-15, implemented.** `--force-recovery-after-secs N` (BENCH ONLY, requires
`--bench-no-sidecar` or startup refuses exit 2 -- so it can never end up armed
against a real, sidecar-bound deployment asset) arms a
`dex_loop::health::ForceRecoveryTrigger` that calls the new
`HealthMonitor::force_recovery` once, N seconds after the stream starts.
`force_recovery` shares `tick`'s budget and position-baseline reset via a
factored-out `issue_recovery_or_escalate`, and main.rs routes BOTH an organic
tick's decision and a forced probe's decision through the same new
`act_on_health_action` -- so a forced probe drives the identical mpv-facing
mechanics (`mpv_command_async(loadfile ... replace)`, absorbing the resulting
`END_FILE(reason=stop)`) a real stall would, not a look-alike. 11 pure-logic
tests (`src/health.rs`) cover the trigger and the shared-budget/baseline-reset
behaviour on the Mac. `tests/cli.rs` adds four automated tests proving the CLI
gate, the missing/malformed-value refusals, and the loud arming warning --
all within this file's mandatory `--opt vid=no --opt aid=no` rule, so none of
them let the trigger actually fire (vid=no/aid=no makes mpv reach "nothing to
play" well under a second in, before a fired trigger could ever be
distinguished from one that never got the chance). The live-fire scenario this
task exists for -- real decode, a real health-check tick observing "healthy",
the forced recovery firing and being survived -- needs exactly the real,
unbounded decode that rule keeps out of the automated suite, so it is a fifth
test, `force_recovery_survives_against_real_mpv`, marked `#[ignore]` with the
exact command and expected stderr sequence to run manually on a Pi with
libmpv.

**2026-08-15, wired into CI as its own gate.** The test above sat `#[ignore]`d
and manual-only since it landed, which meant the C1 fix had zero automated
protection -- nothing would have caught a regression short of someone
choosing to run it by hand. `debian:trixie`'s software HEVC decoder turned
out to make the test's one environment-dependent ingredient (real decode)
available in the *existing* build container: no device, no DRM, no GPU
needed, because `--no-defaults` already skips the whole Pi hwdec/DRM option
set. A new CI step in `.github/workflows/dex-loop-deb.yml` (`build` job,
after `Test`, before `Build package`) now runs the ignored test by exact
name (`cargo test --release --test cli -- --ignored --exact
force_recovery_survives_against_real_mpv`), and asserts `1 passed` in the
output so a renamed/deleted test fails loud instead of leaving the step
green on "0 passed" (libtest's exit code for a filter matching nothing).
`#[ignore]` itself was kept, not removed -- a Pi mid-soak's plain
`cargo test` must never pick up a full real-decode run.

The test was hardened at the same time: deadline raised 15s -> 30s, and a
new assertion added that no organic *second* "attempting in-place recovery"
fires -- the three original assertions (still alive, attempt 1/ logged,
absorption logged) are all satisfiable by a process whose event loop wedged
solid right after absorbing the stop, alive but not actually playing. If
time-pos genuinely resumes advancing post-recovery, the health monitor never
sees a second qualifying stall inside the deadline; if it does not, a wedge
would produce an organic attempt 2/ around t=23s (trigger at 3s, ~10s tick
cadence) -- inside the new 30s deadline, outside the old 15s one.

**Non-vacuity proven, not just asserted (both runs are in Actions history):**
- **Green, the gate as shipped:** run
  [31909711165](https://github.com/KTE/dex/actions/runs/31909711165) --
  `build` job, step "C1 live-fire (forced recovery vs real mpv)":
  `test force_recovery_survives_against_real_mpv ... ok`,
  `test result: ok. 1 passed; ... finished in 30.01s` (ran the full
  deadline, i.e. the process was still alive when killed -- the success
  shape). Whole workflow green.
- **Red, mutation-kill (scratch commit `aa76e0b`, reverted by `74a4e6a`):**
  disabled the `recovery_stops_pending` increment in `act_on_health_action`
  -- the actual C1 fix -- while keeping the parameter formally "used" (`let
  _ = *recovery_stops_pending;`) so Clippy's `-D warnings` gate would not
  fail the build for an unrelated reason first. Run
  [31910026512](https://github.com/KTE/dex/actions/runs/31910026512): the
  `Clippy` and `Test` steps both stayed **green** (this mutation is invisible
  to the pure-logic suite -- `act_on_health_action` is never unit-tested
  directly), and the new "C1 live-fire" step went **red**, in 3.11s, with:
  `attempting in-place recovery 1/3` immediately followed by `dex-loop:
  FATAL: playback ended (reason=2, error=success) -- an endless stream must
  never end; exiting so the supervisor restarts`, and the test's own
  assertion failure `left: Some(1) right: None`. That is C1's exact death,
  reproduced in CI, on CI hardware, caught by nothing except the new step.
  Reverted immediately after in `74a4e6a`; the revert re-ran green
  ([31910191758](https://github.com/KTE/dex/actions/runs/31910191758)).

**What this closes and what it does not.** CI now proves, on every push,
that the process survives its own tier-0 recovery and that time-pos resumes
advancing afterwards -- under software decode with `vo=null`, no display. It
does **not** prove the picture comes back on real hardware: `hwdec=drm`,
`gpu-hwdec-interop=drmprime-overlay`, and the swapped DRM plane assignment
are all skipped via `--no-defaults` and none of them are exercised by a
container with no DRM and no GPU. That claim is still the on-Pi bench
checklist item's job (below), unchanged by this addendum.

**A narrower residual gap even within what CI can see, stated rather than
assumed closed:** the recovery-2/-absence assertion needs the event loop to
still be alive and ticking (`mpv_wait_event` waking on its timeout,
`HealthMonitor` still being ticked) even where it never observes a second
stall. A process whose event loop wedged COMPLETELY right after the absorb
-- `mpv_wait_event` itself never returning again -- would produce neither a
second `attempting in-place recovery` line nor an exit, and every assertion
in the test would pass vacuously. Closing that fully needs a positive
post-recovery signal (e.g. a bench-only per-tick "healthy" log line while T7
is armed, asserted present at least once after the absorb); this is a small,
plausibly cheap follow-up, deliberately not implemented in this pass so it
does not get silently claimed as already covered. See `tests/cli.rs`'s doc
comment on `force_recovery_survives_against_real_mpv` for the same note in
context.

**2026-08-17, on-Pi live-fire against the real display -- T7's actual job.**
Ran the forced probe against **real** hardware for the first time: the full
Pi 4 zero-copy path (`hwdec=drm`, `gpu-context=drm`,
`gpu-hwdec-interop=drmprime-overlay`, the DRM plane swap), not CI's
`--no-defaults --opt vo=null` software-decode stand-in, which skips all of
that by construction.

```
dex-loop ~/bench/loop4k.265 --bench-no-sidecar --fps 30 --mode 3840x2160@30 --force-recovery-after-secs 15
```

Mirrors the real deployment argv (`deploy/dex-loop.service`'s
`--mode 3840x2160@30`, defaults ON) plus the two bench-only escape-hatch
flags T7 requires. `~/bench/loop4k.265` -- not `/opt/dex/loop.265` -- barcode-
verified 0..89 with zero nulls against the CURRENT geometry (`sha256
8eb4bfac...`) before the run, via `bin/capture.mjs --source` against the file
directly. Two runs, each bounded by `timeout` (100 s, then 660 s) so nothing
was left holding the display unattended; both on Pi build `61f3600d553d`
(`dex-loop --version`, stderr):

- Forced recovery fired exactly once, ~15 s after the loadfile request, in
  both runs. `END_FILE(reason=stop)` absorbed as expected -- C1's exact fix
  path -- and the demuxer/decoder re-initialized cleanly with no errors.
- Zero FATAL/panic lines in either run. Zero *organic* second "attempting
  in-place recovery" -- across 84 s (run 1) and ~10 min (run 2) post-recovery,
  the health monitor's own ~10 s ticks never re-flagged a stall.
- A direct, sampled answer to the residual gap two paragraphs up (not the
  permanent instrumented signal proposed there, but real evidence for real
  runs): CPU sampled straight from the Pi process, not inferred from log
  silence, was ~25-27% sustained across multiple post-recovery samples with
  `TIME` climbing steadily between them -- what continuous realtime 4K decode
  actually costs on this Pi. A wedged event loop would read ~0% and flat
  `TIME`. `vcgencmd get_throttled`=`0x0` throughout; temp 43.8-45C.
- Run 2's own 10-minute heartbeat landed ~9m45s after the recovery and closes
  the residual gap even more directly than the CPU sampling above: `wraps=198
  uptime=600s temp=45.2C frame-drops=0 vo-delayed=0 pos=584.0s pos-age=0s`.
  `pos-age=0s` is the load-bearing field -- it is time-since-last-observed-
  position-sample, so a wedged `mpv_wait_event` (the exact failure shape the
  residual-gap paragraph above describes, where every log-based assertion
  passes vacuously) would show a large, growing `pos-age`, not `0s`. `wraps`
  tracking 600s at ~3.03s/wrap (mid-recovery re-init cost included) and
  `frame-drops=0`/`vo-delayed=0` from mpv's own counters corroborate the same
  conclusion from a second, independent instrument.

**What this closes:** the recovery's mpv-facing mechanics (`loadfile ...
replace`, the `END_FILE` absorption, the VO reconfigure) were exercised
against the actual Pi 4 zero-copy path for the first time and survived, on
every signal reachable from the Pi side -- strictly more than CI proves,
since CI's software path never touches DRM/hwdec/the plane swap at all.

**What remains unproven, stated rather than assumed closed:** whether the
*picture itself* reappeared on the physical display was **not** independently
witnessed. The Cam Link instrumentation this task exists to use
(`bin/capture.mjs`, `scripts/record-capture.sh`) could not be exercised this
session -- every attempt (8, over roughly 15 minutes, both single-frame grabs
and the lossless band recorder) hung indefinitely inside ffmpeg's
AVFoundation device-open call, Mac-side, after ruling out the usual suspects:
`check-capture-link.sh` passed (SuperSpeed, 5000 Mb/s) before and after; the
device correctly reported its one supported mode as `3840x2160@30`, meaning
it *was* receiving a valid signal from the Pi; and no other app held it
(QuickTime Player, which was running, was quit and the hang persisted
unchanged). Confirmed **not Cam-Link-specific**: the Mac's own built-in
FaceTime HD Camera hung identically on the same host, at the same time, once
asked for an actual capture (a supported-pixel-format probe on it returned
fast, matching the Cam Link's own fast probe failures -- only the real
capture attempts, on either device, hung). So this is a host-wide AVFoundation
capture-session wedge, not a Cam-Link/USB-link problem -- ruling out re-
seating the dongle as a fix. The hang sits below ffmpeg, in the CoreMediaIO
daemon stack (`VDCAssistant`/`UVCAssistant`/`cameracaptured`, owned by system
user `_cmiodalassistants`); the standard fix -- bouncing those daemons, or a
reboot -- needs `sudo` or console access, unavailable non-interactively this
session. So: the CI-vs-real-hardware gap this task exists to close is
**narrowed, not closed**. Process
survival and continued, correctly-costed resource activity are now proven on
real hardware; the picture's actual return still rests on inference (a
healthy, ticking event loop with a locked DRM plane has no known way to
present a black screen instead), not direct observation. A re-run with a
cleared capture path is the natural follow-up -- nothing about the player,
the flag, or the procedure above needs to change for it.

**Incidental fix, found en route, unrelated to T7 itself:**
`scripts/record-capture.sh`'s device-name lookup (`ffmpeg -list_devices
true`) always exits 251 (ffmpeg's own behaviour, after printing the list) --
under the script's `set -euo pipefail` that non-zero code was propagating
through `NAME=$(... | awk ...)`'s command substitution and killing the
script via `set -e` before it ever recorded a frame, unconditionally, every
time it was run this way. Fixed with a trailing `; true` inside the
substitution (awk's own exit status -- the thing that actually reflects
whether the name was found -- is unaffected). Did not turn out to be this
session's actual blocker (the deeper AVFoundation hang was), but it was a
real, previously-latent bug in the bench tooling and is fixed now.

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
