# From experiment to dex-os: finishing dex-loop and shipping it

**Written** 2026-08-15, after the 4K HEVC perfect-loop experiment produced a working
player, a Debian package and a green CI pipeline.

**End state:** a dex-os image that plays 4K artwork on a Raspberry Pi using our own
player, installed as a package, driven by `pi_video_looper`.

**Route:** finish the sprint and cut an RC (A) → device hardening (B) → promote the
crate to a package (C) → integrate with `pi_video_looper` (D) → new dex-os RC (E).

Phases A and B are independent and can run in parallel. C is a prerequisite for D
and E. **E contains the one genuinely load-bearing unknown** — read "The crux"
before scheduling anything.

---

## The crux: dex-os is buster, the player needs trixie

`packages/dex-os/pi-gen-config.env` says `RELEASE="buster"`. dex-loop needs
**trixie**, and not incidentally — every mechanism the experiment relies on arrived
long after buster:

| Needs | Why |
|---|---|
| `rpi-hevc-dec` + V4L2 stateless request API | the 4K HEVC decode path itself |
| libmpv 0.40 | `--gpu-hwdec-interop=drmprime-overlay`, and the `END_FILE(reason=stop)` behaviour F1's recovery rests on |
| trixie's Pi-patched ffmpeg | hardware decode from distro packages, no vendoring |

So **"4K on dex-os" and "dex-os stays on buster" cannot both be true.** This was
already visible in the project's own framing — Max, 2026-08-13: *"i can stay old
hardware and buster forever, the devices are not network connected. but 4K is
becoming a must for quality."* Those two sentences are in tension, and 4K is the one
that wins; the buster security caveat
(`projects/dex/README.md:137-145` in home-workspace) gets retired as a side effect.

**Decide this before Phase E is scheduled, not during it.** Options:

1. **Rebase dex-os onto trixie.** Port the quilt patches to pi-gen's trixie branch.
   The honest answer, retires the buster caveat, and is the largest single unknown
   in this plan — the patch set has not been touched since 2024.
2. **Two images.** Keep buster for legacy 1080p players, add a trixie image for 4K.
   Cheaper to start, doubles maintenance forever.
3. **Skip dex-os for 4K.** Plain Debian trixie + our `.deb`, which is exactly what
   was verified working on the bench today. Least work, least product.

**DECIDED (Max, 2026-08-15): option (2), two images.** `buster` stays as the
legacy/HD line for existing players; a `trixie` line carries modern/4K. The cost is
real and accepted — two bases to maintain — and it buys not having to migrate
working legacy installations to get 4K on new ones. The buster security caveat
therefore persists for the legacy line and is retired only where the trixie line
lands.

### Does the trixie line run on a Pi 5?

**Decode: almost certainly yes. Presentation: unknown, and that is the axis that
cost this experiment the most.**

The same driver covers both SoCs — `rpi-hevc-dec` is a stateless V4L2 decoder for
**BCM2711 and BCM2712**, and BCM2712 does HEVC 4K60 in hardware. So the decode side
of our stack (ffmpeg V4L2-request hwaccel → the same controls) should port directly,
and a Pi 5 may well clear 4K60 where the Pi 4 measured only 0.753x realtime.

What is genuinely untested is everything *after* decode:

- **The zero-copy plane path.** Our whole performance result rests on the decoder
  emitting SAND-tiled NV12 that the display scans out *natively* —
  `--gpu-hwdec-interop=drmprime-overlay`, with `--drm-draw-plane=overlay` and
  `--drm-drmprime-video-plane=primary` swapped from mpv's defaults. That was
  measured against BCM2711's HVS. The Pi 5's display pipeline differs.
- The crate already flags this. `main.rs` on the plane swap: *"Verified on this Pi 4
  + kernel; re-verify after a kernel upgrade or on any other DRM driver."* A Pi 5 is
  another DRM driver.
- The `+rpt2` ffmpeg criterion (story open point) is explicitly about the Pi 5 and
  remains unresolved; trixie ships `+rpt1`.

**So: yes, it needs trying — but the test to run is the interop, not the decode.**
`scripts/probe.sh` plus `measure-rate.sh` answer it in an afternoon, and the failure
mode to expect is the one from 2026-08-13: realtime-looking throughput with the
frame quietly taking a slower path. Budget a Pi 5 leg as its own task, not as a
footnote to the trixie image.

---

## Phase A — finish the sprint, cut an RC

### A1. Confirm CI is green end to end

The pipeline has never completed all four jobs. Run 3 fixed the cache-warm guard;
run 4 is the first that can pass `build → lint → deny → lifecycle`.

- Verify: all four green on `experiment/4k-hevc-perfect-loop`.
- The `lifecycle` job has still never executed successfully. Until it does, the
  install/remove/purge assertions are unproven in CI.

### A2. F9 — the heartbeat's blocking risk

**The one real deferred defect, and it should not ship in an RC.** Two of three
adversarial reviewers flagged it independently as SUSPECTED MAJOR, and F1 sharpened
it: the heartbeat performs a synchronous property read, which can block on a wedged
core — the exact hazard F1's async design was built to avoid. F1 fixed the health
check and left the heartbeat on the old pattern.

- Files: `dex-loop/src/main.rs` (heartbeat call site), `dex-loop/src/heartbeat.rs`.
- Apply F1's own resolution: read cached values from `MPV_EVENT_PROPERTY_CHANGE`,
  or move to `WatchdogSec` + `sd_notify` and let systemd own liveness.
- Verify: unit tests for the pure logic; on-device confirmation that heartbeats
  still carry `frame-drops`/`vo-delayed`.

### A3. T7 — live-fire recovery test

F1's recovery path has **never executed against a live mpv**. The C1 bug (recovery
killed the process on its first step) was found by code review, not by a test, and
nothing today would catch its return.

- Add `--force-recovery-after-secs <N>`, a hidden/bench-only flag that triggers
  tier-0 recovery on a timer.
- Test: start playback, force recovery, assert playback continues and the process
  survives — the exact thing C1 broke.
- This is new harness infrastructure; budget accordingly.

### A4. 24 h soak on the shipping build, with the real asset

The gate Max set for M5. Note what has *not* been soaked: every soak so far predates
F1, the C1 fix, mandatory sidecars and the package, and used `--bench-no-sidecar`.

- Run the packaged binary under systemd, from `/opt/dex`, with the real artwork.
- Instrument with `scripts/soak-monitor.sh` (run_id + header guard landed today).
- Pass bar: zero `frame-drops`, zero `vo-delayed`, flat RSS, no throttle bits.
- **Do not start it before A2 and A3 land** — soaking code that is about to change
  measures the wrong artifact.

### A5. Merge and tag

- Merge `experiment/4k-hevc-perfect-loop` → `main`, keeping the experiment directory
  intact. The SPEC, the LOG and the Failed Attempts table are the record of *why*
  the player is shaped this way; the crate alone does not carry that.
- Tag `dex-loop-v0.1.0-rc1`. The CI workflow already triggers on `dex-loop-v*`.
- Verify: the tag build produces an installable `.deb` and the artifact is attached.

---

## Phase B — device hardening (parallel with A)

Both belong to M5's card build and both are device-level rather than code.

### B1. F5 — read-only rootfs

A gallery device is switched off at the mains. Today that can corrupt the card.

- `overlayroot` or a pi-gen read-only configuration; `/opt/dex` stays readable.
- Verify: pull power 20× mid-playback, confirm the card still boots and the player
  starts. This is a test that must actually be *performed*, not reasoned about.

### B2. F6 — baked EDID / forced mode

Today's session produced the exact failure this prevents: a stale
`video=HDMI-A-1:3840x2160@30` in `cmdline.txt` pointed at a display that cannot show
4K, which would have looked like a player fault. A Pi that boots before its display
reads no EDID and lands on a 1024×768 fallback that never corrects itself — and for a
mains-switched device that is the normal case.

- Write the intended mode per device, deliberately, at install time.
- Note the mode is now **per display**: 2560×1440@59.95 for the U2719DC.

---

## Phase C — promote to `packages/`

### C1. Decide the name — OPEN, with a direction (Max, 2026-08-15)

Earlier dex thinking pointed at **`dexd`** or simply **`dex`**: *the one binary we
run that manages everything*, potentially absorbing `pi_video_looper`'s features —
USB copy-in, **transcode on ingest** (Milestone 3), playlist and timing (Milestone 4).

That reframes the decision. If the endpoint is one daemon, then this crate is not a
sibling of `pi_video_looper` but its **replacement**, and Phase D becomes a stepping
stone rather than the destination. Two consequences worth deciding deliberately:

- A name like `dex-player` locks in "plays video" and would have to be migrated again
  when it grows ingest and playlists. `dex`/`dexd` does not.
- But a Debian package called `dex` claims a very general name for something that
  today only loops one file. Shipping `dexd` early and growing into it is the
  cheaper order; renaming a package that devices already carry is a migration.

### C1b. Original note

`dex-loop` was an experiment name. In `packages/` it sits beside `dexd`,
`pi_video_looper`, `dex-os`, and its job is narrower than "loop": it is the *player*.
Candidates: keep `dex-loop` · `dex-player` · `dexplay`. The name also becomes the
Debian package name, the binary name and the systemd unit, so changing it later is a
migration rather than a rename.

### C2. Move the crate

- `git mv experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop packages/<name>`.
- **Leave the experiment directory in place** with a pointer. The experiment is the
  evidence; the package is the product. Moving the evidence into the product is how
  the reasoning gets lost.
- Update: CI `CRATE_DIR` and path filters, `make-sidecar.sh`'s `CRATE_DIR`, the
  REUSE root, and `deny.toml`'s location.
- Verify: CI green from the new path; `cargo deb` still builds; `reuse lint` passes.

### C3. Bench harness stays put

`scripts/` and `bin/` are the *instrument*, not the product. They stay in the
experiment directory. The only coupling is `make-sidecar.sh` → the crate, which C2
already handles.

---

## Phase D — `pi_video_looper` integration

### D1. The architectural tension, first

`pi_video_looper` loads a backend by name (`video_looper.py:137`,
`importlib.import_module('.' + module, 'Adafruit_Video_Looper')`) and expects
`create_player(config, screen, bgimage)` returning an object with
`supported_extensions()`, `play(movie, loop, vol)`, `is_playing()`, `stop()`.

That interface assumes a player the looper **starts and stops per video**. dex-loop
is built to **never end** — that is the entire trick. The two models are opposed, and
the integration is a decision about *who owns the loop*, not a wrapper:

- **(a) Looper owns it.** `play()` spawns dex-loop, `stop()` sends SIGTERM.
  Simple, matches the existing backends, and correct for dex's real case (one
  artwork, forever): the process starts once and runs for the exhibition.
  **Must ensure the looper does not also try to loop** — its restart-on-exit logic
  would fight dex-loop's internal endless stream.
- **(b) Player owns it.** dex-loop keeps running and the looper feeds it new assets
  over a control channel. Needs IPC dex-loop does not have. Only worth it if
  playlists with gapless transitions become a requirement (that is Milestone 4).

Recommendation: **(a)** now, revisit under M4.

### D2. Write the backend

- `packages/pi_video_looper/Adafruit_Video_Looper/dex_loop.py`, modelled on
  `hello_video.py` (the closest analogue: a spawn-and-wait player).
- `supported_extensions()` → `['265']`; the sidecar travels beside the asset.
- Config key `video_player = dex_loop`.

### D3. Upstream — unchanged from Milestone 2

Open an issue asking whether a PR would be welcome; **PR only on a positive signal**,
and shipping never blocks on a reply.

---

## Phase E — dex-os RC

Gated on the crux decision above.

### E1. Base migration (if option 1)

- Port `packages/dex-os` quilt patches to pi-gen's trixie branch.
- Expect the largest surprise surface in this plan: the patch set has not been
  touched since 2024-04.

### E2. Install the package into the image

This is where the `.deb` earns its place: a pi-gen stage drops it in and `apt`
resolves `libmpv2` at **image build time**, on a workstation, instead of the failure
appearing on a device in a gallery.

### E3. 4K vs HD variants

- The 4K image needs the trixie base; a 1080p image may not.
- Decide whether dex-os ships one image with mode selection, or a 4K variant.
- Ties back to B2: the image should write the display mode deliberately.

---

## Decisions

**Resolved 2026-08-15:**

1. ~~The crux~~ → **two images**: buster for legacy/HD, trixie for modern/4K.
3. ~~Do F9 and T7 block the RC~~ → **both do.** "Fixing all findings comes first."
   The RC therefore ships with the recovery path actually exercised by a test,
   which was the risk in shipping T7 late.

**Still open:**

2. **The package name** (C1). Direction is `dexd`/`dex`; the decision is really
   *when* to claim the general name, not which name.
4. **Who owns the loop** (D1) — recommendation is (a), the looper spawning and
   killing the player. **The `dexd` direction may dissolve this question entirely:**
   if the endpoint is one binary absorbing USB copy, transcode and playlists, then
   there is no looper to integrate with and Phase D collapses into Phase C. Worth
   deciding *before* writing a backend that may be throwaway.
5. **dex-os variants within the trixie line** (E3) — one image that selects a mode,
   or separate 4K and HD images. The two-image decision settles buster-vs-trixie,
   not this.
6. **The `+rpt2` ffmpeg criterion** — still unresolved, and it is now the gate on
   the Pi 5 leg rather than a general concern.

## Also open, tracked elsewhere — pulled in here so the plan is not misleading

These are not Phase A–E work, but they compete for the same evenings:

- **The Pi 5 leg** (see "Does the trixie line run on a Pi 5?"). A day's work with a
  clear test; blocked only by having a Pi 5 to hand.
- **Enclosure design** (dex idea B, 2026-08-15). Today's box test is the first data:
  sealed and passive, 1440p60 at 45 Mbps reached **75.9 C and was still climbing at
  +0.15 C/min after an hour**, with 4.1 C of headroom left. Whatever the final number,
  a sealed passive case is marginal — the design needs venting, a heatsink, or the
  silent fan. Ambient matters too: a gallery warmer than this room eats the margin
  directly.
- **Jumper-set configuration** (dex idea A) — dev/prod toggle read from GPIO at
  startup. Composes well with F5: configuration that needs no filesystem write
  cannot be corrupted by a power cut, and is visible without a keyboard.
- **Capture rig permanent home** — tracked in the home-workspace story. Blocks
  further measurement work more than it blocks shipping.
- **Licence rollout** — `dex-loop` is MIT-0 and REUSE-compliant; `dex-os`, `dexd`
  and `example-content` are not yet, and repo-wide REUSE is blocked on the
  GPL-inherited pi-gen material.

## Deliberately not in this plan

- **F2 tier-2 reboot escalation** — explicitly a future iteration.
- **Milestone 4 (exhibition format: playlists, per-video timing)** — changes the
  answer to D1 and should not be pre-empted by it. Note the `dexd` direction points
  straight at M4, so this may arrive sooner than its milestone number suggests.
- **The >=500-wrap statistical seam run** — an experiment artifact, not a product gate.
  The M5 artwork does not loop, so seam behaviour is not on its acceptance path.

## Deliberately not in this plan

- **F2 tier-2 reboot escalation** — explicitly a future iteration.
- **Milestone 4 (exhibition format: playlists, per-video timing)** — changes the
  answer to D1 and should not be pre-empted by it.
- **The ≥500-wrap statistical seam run** — an experiment artifact, not a product gate.
  The M5 artwork does not loop, so seam behaviour is not on its acceptance path.

