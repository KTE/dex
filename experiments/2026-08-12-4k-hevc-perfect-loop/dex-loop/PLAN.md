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
