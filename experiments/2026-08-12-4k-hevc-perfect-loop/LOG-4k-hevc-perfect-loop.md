---
phase: SETUP
mode: rigorous
started: 2026-08-12
timebox: open-ended
slug: 4k-hevc-perfect-loop
tags: [dex, video, hevc, 4k, raspberry-pi, mpv, seamless-loop]
verdict: null
confidence: null
---

# Lab Notes: 4K HEVC Perfect Loop — does mpv on Pi OS Trixie wrap without a seam?

**Spec:** [SPEC.md](SPEC.md) — harness architecture, asset spec, measurement method,
test matrix, escalation ladder.
**Implementation plan:** [archive/IMPLEMENTATION-PLAN.md](archive/IMPLEMENTATION-PLAN.md)

## Motivation

dex's only *proven* seamless loop is `hello_video`, which requires the legacy Broadcom GLES
stack, which exists only on buster (32-bit, 2019). That pins dex to a Debian that can never be
security-updated and does not run on the Pi 5 at all. It also maxes out at 1080p.

4K is now a hard requirement (Max, 2026-07-26). The 2026-07-26 research dossiers concluded that
**nobody has published a true 4K seamless loop on the modern Pi stack**, and identified the
cheapest experiment that would settle it: an mpv probe on Pi OS Trixie, whose distro ffmpeg
(`+rpt2`) already carries the Pi 5 HEVC patches, so hardware decode works with system packages
and no vendoring.

This experiment runs that probe. It is the gate on every downstream dex milestone: a
`pi_video_looper` mpv backend, transcode-on-ingest, and the dex exhibition format all assume a
player that loops cleanly. If no such player exists, those milestones are built on sand.

## Hypothesis

There exists an `mpv` invocation on Raspberry Pi OS Trixie that **plays a 4K30 HEVC file correctly
and loops it with no seam** — correct playback meaning realtime rate, full frame rate, correct
colour and correct geometry; no seam meaning a wrap-point anomaly rate statistically
indistinguishable from the same run's mid-loop capture noise floor — on **both Pi 5 and Pi 4**,
sustained without drift over 24 hours.

> **Amended 2026-08-13** (SPEC §1, §4.4). The original hypothesis named only the seam. mpv then ran
> at 14.3 fps of a required 30 while reporting zero dropped frames — correct by the old bar,
> useless in fact. **Correct playback is the bar; seamlessness is one clause of it.**

## Success Criteria

- [ ] MUST: **correct playback** — SPEC §4.4 clauses A1–A5 (realtime rate, full frame rate at the
      display, colour, geometry, native resolution). Checked **before** any seam measurement: a
      failure here makes a run **VOID for seam purposes**, not a seam failure
- [ ] MUST: wrap-point anomaly rate indistinguishable from the mid-loop baseline over **>=500
      consecutive wraps** at 4K30 on Pi 5
- [ ] MUST: the *same* config passes on **Pi 4** (SCOPE R4 — a Pi-5-only result does not qualify)
- [ ] MUST: **24 h soak** with no drift in mpv's in-band frame-drop counters
- [ ] MUST: `analyze.mjs` **validated against synthetic logs with injected defects** — a clean
      result from an unvalidated analyzer is not evidence
- [ ] MUST: deliverable is a **literal argv**, not a description — it is what milestone 2's
      `mpv.py` `play()` shells out to
- [ ] SHOULD: **A/B against the dexOS card** (`hello_video`, same source content) on the same
      display shows no difference to the eye
- [ ] SHOULD: **4K60 stretch check** on Pi 5, measured via the weaker 1080p60 capture path

## Fail Condition

For **every** config in the matrix, on **both** boards: either **correct playback (§4.4 A) cannot
be achieved at all**, or the wrap-point anomaly rate exceeds the mid-loop baseline by a
statistically significant margin (p < 0.01 over >=500 wraps).

The first disjunct was added 2026-08-13 and is the live one: if no configuration reaches realtime,
mpv has failed regardless of what its wraps look like. That is a fail, not a void — a void run is
one where the *instrument* was compromised; this is the player itself failing a bar clause.

That is the disproof. Note what it is *not*: "it looked stuttery" is not a fail condition, and
neither is "one wrap out of 500 was bad" — a defect present at one wrap in 500 is indistinguishable
from capture noise, which is exactly why the baseline comparison exists.

## Time Box

**Deadline:** open-ended
**Early Stop:** **2 bench days** without any config reaching baseline on either board -> stop,
declare REFUTED, escalate per Pre-Committed Decisions. Bench days, not calendar days.
**Rationale:** open-ended on the calendar because this is evening/weekend work and a stalled month
means nothing; hard-stopped on *effort* because the failure mode this experiment is most likely to
suffer is an open-ended mpv config grind. The two are not in tension — one bounds wall-clock
patience, the other bounds sunk cost.

## Pre-Committed Decisions

**If confirmed:** freeze the argv verbatim into `SPEC.md` findings. Proceed to milestone 2 — fork
`adafruit/pi_video_looper` (fork, not quilt: a new backend file patches nothing upstream), write
`Adafruit_Video_Looper/mpv.py`, re-run `analyze.mjs` against the integrated backend as a regression
test. GRADUATE the harness into the dex project proper.

**If refuted:** escalate in this fixed order, no re-litigation:

1. **pivid on Pi 4** — cheapest next step, because `example-content` already ships pivid `.json`
   timelines. Settles `status.md` open question #5 either way.
2. **GStreamer segment-seek** (`v4l2slh265dec` + kmssink) — known caveat: Trixie's kmssink is not
   yet zero-copy for HEVC.
3. **Custom C player** — ~2k lines against Trixie system libav, 10-15 person-days, with its own
   5-day tracer stop condition (DOSSIER-2 §5 Rank 1, §6.4).

**Explicitly not permitted on refutation:** continuing to try mpv configurations. That is what the
early-stop exists to prevent.

## Environment

### Mac side — measurement host (recorded 2026-08-12, complete)

| Component | Version |
|---|---|
| Host | `katoz.178.is`, macOS 26.5.1, arm64 |
| Node | v25.9.0 |
| pnpm | 10.28.1 |
| ffmpeg / ffprobe | 8.1 (libx265, libx264, geq, drawbox, avfoundation) |
| shellcheck | 0.11.0 |
| TypeScript (dev only) | 5.9.3 · `@types/node` 22.20.1 |

### Pi side — playback host (recorded 2026-08-13 at the bench)

| Component | Value |
|---|---|
| Host | `dexpi4.local` / `192.168.1.74`, SD card `d05` |
| Board | Raspberry Pi 4 Model B Rev 1.1, revision `c03111` |
| OS | Debian GNU/Linux 13 (trixie) — Raspberry Pi OS **64-bit Lite** |
| Kernel | `6.18.34+rpt-rpi-v8` aarch64 |
| mpv | v0.40.0 |
| ffmpeg | `7.1.5-0+deb13u1+rpt1` — ⚠️ **`+rpt1`, not `+rpt2`** (see below) |
| HEVC decoder | `rpivid` present at `/dev/video19` (HEVC listed as an output format) |
| Thermal | `throttled=0x0`, 42.3 °C idle at 4K30 — no active cooling yet |

- **The `+rpt2` check did not pass as written.** The pre-committed criterion above expects the
  `+rpt2` distro build. Trixie ships `+rpt1`. The criterion was written about the **Pi 5**
  HEVC patches, and this is a Pi 4 decoding through `rpivid`, so it plausibly does not apply —
  but it is recorded as *unresolved* rather than waved through, and must be settled before any
  Pi 5 leg.
- **Display / sink:** the Cam Link 4K itself (not a monitor) — see Capture side.
- **Negotiated mode:** `3840x2160`, RGB 4:4:4, 8 bpc, limited range, `tmds_char_rate` 297 MHz.
- **`hdmi_enable_4kp60` not needed** — 4K30 is 297 MHz, the HDMI 1.4 ceiling. It would only be
  required for a 4K60 leg, which this sink cannot accept anyway.
- **HDMI cable:** micro-HDMI to HDMI, not certification-marked, but empirically proven at
  297 MHz — the same cable drives a real 4K display. It was wrongly suspected during
  diagnosis; see the 2026-08-13 entry.
- **`vc4.force_hotplug` not used.** Instead `video=HDMI-A-1:3840x2160@30` is forced in
  `/boot/firmware/cmdline.txt` (original kept as `cmdline.txt.orig`), which fixes a real
  boot-order fragility rather than just a bench annoyance — see the 2026-08-13 entry.

### Capture side (recorded 2026-08-13 at the bench)

| Property | Value |
|---|---|
| Device | Elgato Cam Link 4K — `avfoundation` **device index 0** ("Cam Link 4K") |
| EDID | mfg `EGA`, product `0x0066`, name `Cam Link 4K`, EDID 1.3 + one CEA-861 rev 3 block |
| Advertised 4K | VIC 95 / 94 / 93 → 2160p **30 / 25 / 24 only**; no 2160p50/60 |
| Colour | RGB 4:4:4, YCbCr 4:4:4, YCbCr 4:2:2 — **no 4:2:0** (no HDMI Forum block; HDMI 1.4) |
| Pixel formats to host | `uyvy422`, `yuyv422`, `nv12`, `0rgb`, `bgr0` |
| **Measured 4K rate** | **~27 fps, not 30** — uniform pacing, not dropped frames |
| USB requirement | **SuperSpeed (5 Gb/s) mandatory** — gated by `scripts/check-capture-link.sh` |

- **The "4K ceiling is 30 fps" assumption is wrong as stated.** It is ~27 fps in practice.
  Measured over 600 frames (22.43 s, scaling linearly from 300 frames / 11.04 s). Inter-frame
  timestamp deltas cluster at 0.0358–0.0373 s with **no bimodal distribution and no doubled
  intervals**, which is pacing rather than frame loss. Requesting `nv12` instead of `uyvy422`
  changed nothing (byte-identical 11.04 s), consistent with the card transmitting 4:2:2 over
  USB regardless of the host-requested format. 4:2:2 at 4K30 needs ~497 MB/s against USB 3.0
  Gen 1's ~450 MB/s practical ceiling; **450/497 = 0.905** against a measured **27/30 = 0.90**.
  Recorded as the best-fitting inference, not as proof.
- **No HDMI passthrough** on this unit — capture and projector A/B necessarily run sequentially.
- **Firmware version:** not read (Elgato exposes it only via their own utility). Not blocking.
- Optional Insta360 X4 check: still not done. Now more interesting than before, since the Cam
  Link cannot deliver a full-rate 4K30 capture.

## Baseline

Two distinct baselines, not to be confused:

1. **The reference loop** — dexOS (`hello_video`, 1080p H.264) on Pi 4. The only true seamless loop
   in Max's possession that is *proven* rather than claimed. Calibrates what "no seam" looks like to
   the eye, on the same display, in the same room. Does **not** calibrate 4K capability.
2. **The capture noise floor** — the mid-loop anomaly rate, measured simultaneously within every
   run. USB capture is not frame-locked to the Pi, so it drops and duplicates frames on its own;
   those artifacts are uniformly distributed, while a real seam is concentrated at the wrap. This
   is the control group, and it is re-measured on every run rather than assumed.

---

## Running Log

### 2026-08-12 15:10 — Experiment framed

Design agreed with Max in session. Mode: rigorous, timebox open-ended with a 2-bench-day early
stop on effort. Branch `experiment/4k-hevc-perfect-loop` created in `~/CODE/dex` off
`origin/main` (tag `v1.0.0-rc.1`) — deliberately *not* off `dex-os-cleanup`, which has newer
unmerged WIP on origin and is orthogonal repo-housekeeping work.

Decisions locked during framing, with reasons, so they are not silently revisited:

- **Staged probe, not a bake-off.** mpv first; escalate only on failure. Cheapest path to a real
  answer (DOSSIER-2 §6).
- **4K30 for measurement, 4K60 as a stretch check.** The Cam Link 4K records 4K at 30 fps, so at
  4K60 half the frames would be invisible to the instrument. 4K30 also avoids the Pi 4's
  `hdmi_enable_4kp60` requirement and its 4K60 4:2:0 chroma limitation.
- **Silent for the MVP, audio recorded as a future constraint.** Do not select a structurally
  silent-only backend. mpv satisfies this for free; it is why `hello_video` cannot be the
  long-term answer even if it were not buster-bound.
- **Fork, not quilt, for milestone 2.** Quilt exists to keep dex's diff against a *moving upstream*
  legible. A new backend file patches nothing — expressing "add one file" as a quilt patch is
  ceremony with no payoff. Upstream is GPLv2 (`LICENSE` present), so a fork is unambiguously
  permitted and stays GPLv2, which dex already is.
- **Upstream contact is post-stabilisation and non-blocking.** Fork -> build -> stabilise -> open an
  issue linking the fork asking whether an mpv PR would be welcome -> PR only on positive signal.
  Shipping never waits on a reply. Max checked activity on
  <https://github.com/adafruit/pi_video_looper/commits/master/> — still low.

Existing material found that materially reduced scope: `packages/example-content` is already a
**multi-player test kit**, not just a video. For each variant it emits the encode plus
player-specific sidecars — `.mp4.h264` elementary stream for `hello_video`, `.json` pivid timeline,
`.html` `<video loop>` for Cog/WPE — at 1s/2s/3s durations. The A/B rig for this exact comparison
was built in 2024. So the planned `make-asset.sh` shrank to two small scripts operating on existing
masters (see SPEC.md §3).

Gap found: **no 4K master exists** (`lossless/` is 1080p only), though
`test-cards/resources/AltekaKard-4K.png` is sitting there for it and `test-cards.aep` is the
source. Resolved as: AE render for realism + procedural generator for the parameter sweep. Sync
burden is near zero because the analyzer depends only on the barcode, never on the content.

Noted, not acted on: three untracked `.h264` elementary streams from 2024-04-20 in the
`example-content` submodule working tree. They are the `hello_video` A/B reference assets and have
been uncommitted for two years. Committing into a submodule is a separate repo's history and Max's
call, not a side effect of starting an experiment.

### 2026-08-12 15:35 — Refinement: AE render moved off the critical path

Max, mid-session: he will handle the 4K re-render himself for **eyeball checks and demos** — the
rotating elements in `test-cards.aep` are tuned for *human* perception of loop quality — but it
**will not be part of the automated experiment**, and the experiment must not block on it.

Correction to the 15:10 entry: both masters still exist, but the dependency changed. The automated
pipeline's input is **only** the procedural master from `make-master.sh`. The AE render is a
parallel, human-facing instrument that can land at any time. This removes the "two things to keep
in sync" concern entirely and shortens the critical path.

Consequence worth recording, because it is the strongest idea in the design and it only became
visible once the two masters were properly separated: burning the barcode into the **1080p H.264**
master as well makes the dexOS card a **positive control**. Synthetic defect injection proves the
analyzer is *sensitive* (it catches planted defects); `hello_video` — the only proven seamless loop
in existence here — proves it is *specific* (it does not invent defects on a known-good loop). A
method with only one of those controls can be confidently wrong in one direction. Both are now in
SPEC.md §4.3 as Controls 2 and 3.

### 2026-08-12 16:20 — Document rename and asset vocabulary corrected

Three corrections from Max after the implementation plan was drafted:

1. `PLAN.md` -> **`SPEC.md`**. It is a spec in the superpowers sense — what is being built and why —
   not a plan. Earlier entries in this log referred to `PLAN.md`; those links were repaired in
   place, which is a mechanical link fix, not a revision of any observation.
2. The implementation plan moved to **`archive/IMPLEMENTATION-PLAN.md`** — kept with the experiment
   data, deliberately less prominent than the spec and this log.
3. **"master" was the wrong word for the lossless generated original.** I had reached for outside
   vocabulary (considered `plate`, `mezzanine`, `raw source`) when the project already had its own:
   `packages/example-content` calls them **test cards** — `### dex-test-card` under `## animations`,
   lossless originals at `export/lossless/test-card-1s-1080p.mov`, encodes as
   `dex-test-card-2s-1080p-h265.mp4`.

Adopted the existing convention rather than a new one: `make-test-card.sh`, lossless output at
`out/lossless/test-card-1s-2160p30.mkv`, encodes as `dex-test-card-1s-2160p30-h265.mp4`. The
existing `{duration}-{resolution}-{codec}` pattern extends to 4K as `2160p30` / `2160p60`, because
frame rate now distinguishes variants where at 1080p it did not.

Worth recording as a lesson, not just a rename: the instinct to coin a term is a signal to go and
look for the one that already exists. The 2024 example-content kit had answered this question and I
did not check before proposing four alternatives.

### 2026-08-12 17:05 — Insta360 X4 raised as an alternative capture instrument

Max has a Cam Link 4K **and** an Insta360 X4. Assessment, not yet tested:

**In its favour.** The harness is source-agnostic — `capture.mjs --source` takes any ffmpeg
input — so a webcam-mode X4 costs nothing to try. Oversampling genuinely helps: capturing 30 fps
content at >60 fps means each source frame appears several times, so a held frame at the wrap
becomes *more* visible, not less. And it would let **4K60 be measured properly** rather than taken
on trust, which the Cam Link's 4K30 recording ceiling otherwise prevents.

**Against.** It reintroduces exactly what the Cam Link removes — rolling shutter, no frame lock,
moiré against the panel, exposure drift. The mid-loop control still absorbs that statistically, but
the noise floor rises, so more wraps are needed for the same confidence.

**Possible disqualifier.** It is a 360 camera. Dewarping a fisheye projection could distort the
barcode cell spacing enough that centre-sampling lands on the wrong cell — and the sync cells
detect *contrast*, not *misalignment*, so a distorted read would produce confidently wrong indices
rather than `null`. That is the worst failure mode available: silent, plausible, wrong.

**Test, ~10 minutes at the bench:** point it at a screen playing a barcoded test card, run
`capture.mjs` against it, check the decoded sequence is monotonic. If it is garbage, dewarping
killed it and the Cam Link stands. Deliberately *not* blocking anything.

### 2026-08-12 17:40 — Harness built and validated offline; advancing to SETUP

All eight implementation tasks complete. **42 tests pass, 5 shell scripts shellcheck-clean,
`tsc --noEmit` clean.** Nothing here has touched a Pi or a capture device — which was the point:
the instrument is validated before the bench session, so a surprise at the bench is attributable to
the player rather than to the measurement.

The end-to-end dress rehearsal (`test/e2e.test.mjs`) runs the real pipeline — generate, burn,
encode, capture-from-file, analyze — and confirms `PASS` on a genuinely seamless source, `FAIL` on
a planted held-frame seam, and `FAIL` with a non-zero `decodeFailures` count on a planted black
frame.

Four findings during implementation that changed the design or corrected the spec:

1. **`sin(2*PI)` is not zero, and it broke the bit-exact wrap.** The generator's rotation angle
   `2*PI*N/PERIOD` made frame `PERIOD` only *mathematically* equal to frame 0; in IEEE754,
   `sin(2*PI) = -2.45e-16`, which flipped **95 of 61,440 pixels** by one luma level. Fixed by
   wrapping the counter — `mod(N,PERIOD)` — before the trig, so both frames evaluate the identical
   expression. Exact by construction rather than by luck. Without this the experiment would have
   been measuring its own asset.
2. **The spec's fail-condition prose was wrong.** It asserted that "a defect present at one wrap in
   500 is indistinguishable from capture noise." Against a *perfectly clean* mid-loop sample that is
   false: one wrap anomaly in 600 gives z=5.4, p~7e-8. And the strict behaviour is correct — at a
   1 s loop that is a visible stutter every ten minutes for six weeks. The claim is only true once a
   real noise floor exists. Both cases are now paired tests, so the control is demonstrated rather
   than asserted. SPEC.md §4.3 corrected accordingly.
3. **ffmpeg 8.1's csv writer rejects an explicit space separator** (`Failed to parse option string
   'p=0:s= '`) and — worse — emits nothing rather than failing, so the test card's dimensions
   vanished silently into the pivid sidecar as `"mode": [, , 30]`. Now uses the default comma
   separator with `IFS`, and asserts the dimensions are non-empty.
4. **Node 25 rejects a bare directory for `--test`.** `node --test test/` throws MODULE_NOT_FOUND.
   Every individual test file passed, so this would have shipped as a permanently-broken
   `pnpm test` had the aggregate script never been run. Now `node --test test/*.test.mjs`.

**Not done, and blocking nothing:** the three untracked `.h264` files in the `example-content`
submodule working tree. The harness now generates its own barcoded H.264 positive-control asset via
`encode-variants.sh --h264`, so Control 3 no longer depends on them.

### 2026-08-12 17:25 — Procedural test card demoted; the real card was always better

Max, on seeing the generated 4K assets: *"whoa, those test videos are way too psychedelic. is there
a technical purpose for the animation or are the barcodes enough?"* — and proposed overlaying the
barcode onto the existing example content instead.

Correct on both counts. The animation *does* have a technical purpose — decoder load, and a
human-visible seam — but `test-cards.aep` already serves both, better. Frames extracted and
inspected: it has a burned-in frame counter, three rotating sweep hands, resolution wedges, a
checkerboard border, greyscale ramp and colour bars. It looks like an instrument. The procedural
plane-wave card looks like a screensaver.

Verified rather than assumed: the existing card's wrap is matched. The top-left hand is at 12
o'clock on frame 0 and 3 o'clock on frame 15 — 90° in 15 of 60 frames = 6°/frame = exactly one
revolution, so the 59→0 step is +6°, identical to every other step.

And the combination is free, because `add-barcode.sh` was built content-agnostic: burned the strip
onto `test-card-2s-1080p.mov` and **all 60 frames decoded exactly**. Nothing had to change.

**Decision:** the AE card is the bench asset at every resolution; `make-test-card.sh` is demoted to
a unit-test fixture (keeps the suite fast and free of committed binary assets, and never reaches a
screen). Max is exporting the 4K AE comp now, so the 4K measurement no longer waits on anything.

Two things found while checking, both recorded in SPEC.md §3.2 because they will matter at 23:00 on
a bench:

- The barcode strip covers the **top checkerboard border** and clips the top-centre arrow. Fine —
  that border is duplicated on all four edges.
- **The card's own counter resets at exactly the wrap point.** That is a deliberate, large, visible
  discontinuity sitting precisely where an accidental one is being hunted. For the eyeball A/B,
  watch the *rotating hands*, not the counter.

The psychedelic 4K assets were deleted rather than kept. The background render of the 10s variants
was killed partway rather than allowed to finish — no point spending 35 minutes on content that was
about to be replaced.

### 2026-08-12 17:30 — `build-bench-assets.sh` added (ninth script, not in the plan)

The plan had six manual steps between a lossless export and a bench-ready asset set. With Max
exporting the 4K card right now, that is six chances to get a flag wrong at the moment it matters
most. One script now does it end to end, and — importantly — **probes the input rather than being
told about it**, so the variant name always describes what the file actually is:

```
scripts/build-bench-assets.sh --input <lossless.mov> [--h264]
```

It derives resolution, frame rate and duration; picks a bitrate by resolution (40M at 4K, 20M below,
both under the Pi 4's ~80 Mbps HEVC ceiling); burns the barcode; encodes HEVC + optional raw H.264 +
pivid/cog sidecars; and then **decodes the barcode back out of the finished encode and fails loudly
if any frame mismatches.** A silent barcode failure would poison every measurement taken with that
asset, so it is checked rather than trusted.

Built from Max's three existing masters, all verifying clean:

| Asset | Frames | HEVC | raw H.264 |
|---|---|---|---|
| `dex-test-card-1s-1080p30` | 30 | 384K | 346K |
| `dex-test-card-2s-1080p30` | 60 | 796K | 667K |
| `dex-test-card-3s-1080p30` | 90 | 1.2M | 981K |

The `.h264` files are the **Control 3 positive-control assets** — the same content the proven
`hello_video` loop can play. Note this removes the last reason to touch the untracked `.h264` files
in the `example-content` submodule: these are generated, barcoded, and reproducible.

### 2026-08-12 17:45 — 4K export received; decimation added for the Cam Link's 4K30 ceiling

Max's 4K export: `test-card-3s-4K.mov`, qtrle rgb24, **3840x2160 @ 60fps**, 180 frames, 436 MB.

The 60 fps creates a problem that would have been ugly to diagnose at the bench: **the Cam Link
records 4K at 30 fps.** Pointed at 4K60 content it captures every *other* frame, so the analyzer
sees indices stepping by 2 throughout — every transition reads anomalous and wrap detection breaks.
That is an instrument limit, and letting it masquerade as a player property would have wasted an
evening.

So the two 4K assets take different capture paths, and `build-bench-assets.sh --target-fps` was
added to produce the first:

| Asset | Capture path | Role |
|---|---|---|
| 4K30, decimated 60->30 | Cam Link at 4K30 — one captured frame per displayed frame | **Primary** |
| 4K60, native | Cam Link at 1080p60 — full rate, reduced resolution | Stretch check |

Decimation is legitimate *for this content*: synthetic graphics, no motion blur, so every other
frame is an exact 30 Hz sampling of the same motion and the rotations still complete over the loop.
Non-integer factors are refused rather than rounded.

### 2026-08-12 18:00 — Barcode moved: centred, and exactly on the card's grid

Max, seeing the first build: *"can we put the barcode centered and exactly on the grid?"* — with the
lower third marked.

Right call, and it fixed the two placement complaints from the 17:25 entry at the same time: the
top-edge strip had been covering the checkerboard border and clipping the top-centre arrow.

**The grid was measured, not eyeballed.** Extracted a clean frame, scored every column and row
against the field median, and read off the line positions: **50 px cells at 1080p, lines at
x = 10 + 50k and y = 40 + 50k.**

That gives an exact fit — 18 cells of one grid square each:

| | rect | centred? | on grid? |
|---|---|---|---|
| 1080p | `900x50+510+940` | 510 + 450 = 960 ✓ | 510 = 10+10·50, 940 = 40+18·50 ✓ |
| 2160p | `1800x100+1020+1880` | 1020 + 900 = 1920 ✓ | 1020 = 20+10·100, 1880 = 80+18·100 ✓ |

**Unexpected payoff.** Expressing the rectangle as *fractions of the frame* lets both filters be
built from ffmpeg `iw`/`ih` expressions, so neither the burner nor the decoder needs to know the
resolution. `capture.mjs --height` and `add-barcode.sh`'s ffprobe call are both **gone** — one fewer
flag to get wrong at 23:00, and it matters specifically because the 4K60 stretch check is captured
through the 1080p60 path, where source and capture resolutions differ. The old `--height` warning in
the README is deleted rather than reworded.

Verified visually on frame 15: sync-white, sync-black, then four lit cells = bits 0-3 = 15, matching
the card's own `00:15`. 44 tests pass; the geometry now has its own tests asserting centring, grid
alignment, and that no resolution is hardcoded in either filter.

All assets rebuilt. The three 1080p variants verify clean; the 4K pair is rebuilding.

### 2026-08-13 20:50 — First bench session: rig stood up, capture chain proven at 4K30

No loop measured yet. This session was entirely about making the instrument trustworthy, and
it found two instrument faults before a single frame of the bench asset was played.

**Standing up the Pi.** Raspberry Pi OS trixie 64-bit Lite on card `d05`, SSH-only. Lite
deliberately: the desktop image ships the labwc Wayland compositor, which would sit between
the decoder and HDMI scanout and become an instrument-side variable. Lite gives direct
KMS/DRM — the barest stack, and what a real dex player would boot. Full environment table
above. `rpivid` confirmed present at `/dev/video19`, so mpv cannot silently fall back to
software decode, which was the precondition most likely to poison a run.

**Fault 1 — 4K capture produced a flat green image.** Diagnosis in order:

1. ConsoleLink showed green at 4K, and a working preview at 1080p. Ambiguous across four
   candidate causes (app, driver, card, source).
2. Bypassing ConsoleLink and reading the raw UYVY bytes collapsed it to one: **all zeros**
   (`Y=0, Cb=0, Cr=0` converts to exactly that green). Not a misinterpreted image — an absent
   one. `avfoundation` also reported an **empty supported-mode list**, i.e. no input lock.
3. I then built a confident and **wrong** diagnosis on the HDMI side — see Failed Attempts.
4. Actual cause: the Cam Link had enumerated at **USB 2.0** (`Device Speed = 2`,
   `UsbLinkSpeed = 480000000`) behind an `Anker USB2.0 Hub`. The card advertises its available
   *input* modes according to USB bandwidth, so without SuperSpeed it stops offering 4K
   entirely. Max found it by changing ports, then fixed it properly with a USB 3 dongle.

The failure is **intermittent** — the same port renegotiated between SuperSpeed and USB 2.0
twice within one session, minutes apart. An undetected mid-run drop would zero every captured
frame, which `analyze.mjs` would report as total decode failure: indistinguishable from a
catastrophic player fault. So it is now a gate, `scripts/check-capture-link.sh`, run **before
and after** every capture — a pre-flight check alone cannot see a mid-run drop.

**Fault 2 — the "30 fps 4K ceiling" is really ~27 fps.** Measured, uniform, and a property of
the card over USB 3.0 Gen 1 rather than dropped frames. Numbers and reasoning in the Capture
side table. Consequence for the experiment: a systematic ~10 % frame deficit that belongs to
the instrument. Barcode-index analysis tolerates gaps by construction, so this is a design
input, not a blocker — but by the noise-floor reasoning in Success Criteria, a higher floor
makes the test **less** sensitive, so runs must be sized accordingly, and a ~10 % shortfall at
4K must never be read as a player defect.

**Fault 3 — boot-order fragility, and it is a dex production bug, not a bench annoyance.**
After an accidental reboot and a re-flash, the Pi came up at **1024×768** (`tmds` 78.75 MHz) —
the DRM fallback used when no EDID is read at boot. The connector later read the Cam Link's
EDID correctly (byte-identical to the earlier dump), but the console had already set its mode
and does not re-modeset on its own. Forcing a re-probe restored the 4K mode list without
fixing the active mode.

This matters beyond the bench: **dex players are switched off at the mains.** If the display
or projector is not awake when the Pi boots, the player lands on a 1024×768 fallback and stays
there. Fixed deterministically with `video=HDMI-A-1:3840x2160@30` in
`/boot/firmware/cmdline.txt` (original kept as `cmdline.txt.orig`), which is the
production-correct answer rather than `vc4.force_hotplug`.

**End state:** Pi boots straight to 3840×2160 at 297 MHz, `throttled=0x0`, 42.3 °C. Full chain
verified end to end — Pi 4 trixie → micro-HDMI → Cam Link → USB 3 SuperSpeed → `avfoundation`
→ `ffmpeg` — with the USB gate passing before and after. mpv v0.40.0 and ffmpeg installed.

**Next:** `hello_video` positive control, then the mpv config matrix against
`dex-test-card-3s-2160p30-clouds.mp4` with `--loop-length 90`.

### 2026-08-13 21:40 — mpv does not reach realtime on Pi 4; no seam measured yet

**Still no seam measurement.** The run was blocked upstream by something the pass bar never
thought to require: *the player must actually play at realtime*. It does not.

**Headline (preliminary, see caveats):** the best mpv configuration found reaches **14.3 fps**
against a 30 fps requirement — `ratio 0.476`. Every other configuration is worse.

| Path | Decode | Rate |
|---|---|---|
| `ffmpeg -hwaccel drm` (no player) | hardware, zero-copy | **41 fps (1.36x)** ✅ |
| mpv `--vo=drm --hwdec=drm-copy` | hardware **+ 4K copy to RAM** | 14.3 fps (0.476x) |
| mpv `--vo=drm --hwdec=no` | software | 5.9 fps |
| mpv `--vo=gpu --hwdec=drm` | software (hwdec silently declined) | 5.2 fps |
| mpv `--vo=null --hwdec=drm` | software (null VO cannot take DRM frames) | 12.1 fps |
| mpv software, ffmpeg CLI | software | 11 fps (0.363x) |

**Mechanism.** The HEVC block is not the limit — bare ffmpeg decodes this exact file at 41 fps
through `rpi-hevc-dec`. What mpv cannot do on this stack is engage the **zero-copy DRM_PRIME**
path: `--hwdec=drm` with `--vo=drm` reports `Selected decoder: hevc` (software) rather than a
hardware surface. Hardware decode is only reachable via `drm-copy`, which copies every 4K frame
back to system memory, and that copy becomes the new bottleneck. Attempts with
`--drm-draw-plane=overlay --drm-drmprime-video-plane=primary` did not engage it either.

**Two instruments agree**, which is why this is worth recording despite the caveats below:

- `measure-rate.sh` via mpv's IPC: `ratio=0.476`, effective 14.3 fps
- The **barcode capture itself**: indices advanced 50→71 in ~1.5 s = ~14 index-steps/s

That agreement matters because they share no code path — one reads mpv's own clock, the other
reads pixels off an HDMI capture card.

**mpv's drop counters are useless for this.** Throughout, `frame-drop-count`,
`decoder-frame-drop-count` and `vo-delayed-frame-count` all read **0**. mpv is not dropping
frames; it is presenting every one of them, far too slowly. Max saw exactly this on the
capture preview and described it as "all frames, but in slow motion". Only playback-time
against wall-clock detects it — hence `scripts/measure-rate.sh` as a gate.

**Caveats — this is not a verdict:**

1. The 1080p run was **not a clean control**. The display is forced to 3840x2160, so a 1080p
   file is software-upscaled 4x, which is why it came out *slower* (6.8 fps) rather than
   faster. A real 1080p control needs the output mode set to 1080p, and has not been run.
2. The sink is the **Cam Link, not a monitor**. Whether that affects page-flip timing is
   untested, and it is a difference from any real dex installation.
3. The asset is deliberately heavy (39.7 Mbps, grain 7). The clean 4K card has not been tried.
4. mpv's option space is not exhausted.

**Consequence if it holds.** This is upstream of the seam question: a player at 0.48x has no
meaningful wrap, so M1 cannot be answered against mpv in this configuration. The escalation
ladder's step 1 is **pivid**, which does zero-copy KMS plane scanout — precisely the thing mpv
failed to do here — and the bench asset already ships a pivid `.json` sidecar. That is the
next thing to try, not more mpv flags.

### 2026-08-13 22:00 — Optimising the asset would not help: measured, not reasoned

Max asked whether the playback file should be optimised next. Settled empirically rather than by
argument, by re-encoding the bench asset at a **12.5x lower bitrate** with everything else held
constant (same 3840x2160, same 30 fps, same closed GOP, barcode re-verified 0..89 after encode):

| Asset | Bitrate | mpv `vo=drm hwdec=drm-copy` | ffmpeg decode only |
|---|---|---|---|
| `dex-test-card-3s-2160p30-clouds.mp4` | 39.3 Mbps | 14.3 fps (0.476x) | 1.36x |
| `lowbitrate-2160p30.mp4` | **3.1 Mbps** | **15.2 fps (0.507x)** | 1.43x |

**12.5x less bitrate bought 6% more speed.** So the bottleneck is bitrate-independent, which is
exactly what the `drm-copy` diagnosis predicts: the per-frame copy back to system memory costs
*pixels x frame rate*, and is untouched by how many bits the pixels arrived in. Decode was never
the constraint — it already ran above realtime in both cases.

**Consequence:** encoder settings cannot fix this. What would move it is reducing pixels/second
(lower resolution or frame rate — both off the table, 4K is the requirement) or **eliminating the
copy**, which means a player that scans out DRM_PRIME frames directly. That is pivid, escalation
step 1.

Kept as a fixture rather than deleted: `out/lowbitrate-2160p30.mp4` is the control that makes this
claim reproducible.

### 2026-08-13 22:10 — Real monitor closes both caveats; the wall is 4K itself

Swapped the Pi from the Cam Link to a real display (`AW32A`, 2560x1440@60, TMDS 241.5 MHz).
This was meant to test one caveat and closed two.

| Source | Sink | ratio | effective fps | drops |
|---|---|---|---|---|
| 4K clouds | Cam Link @ 4K | 0.476 | 14.3 | 10 |
| 4K clouds | **1440p monitor** | 0.463–0.486 | **13.9–14.6** | 14–17 |
| 1080p card | 1440p monitor | **0.952** | **28.5** | **0/0/0** |

**Caveat 2 closed — the sink is not the variable.** 14.6 fps into a 1440p monitor against 14.3 fps
into a 4K capture card. Two different sinks, two different output resolutions, same result. The
Cam Link was not distorting anything, and the earlier numbers stand.

**Caveat 1 closed — and the earlier 1080p anomaly is explained.** 1080p source now runs at 0.952
with **zero** reported drops. The earlier "1080p is slower than 4K" result was exactly the
suspected artefact: the display was forced to 3840x2160, so a 1080p file was being software-
upscaled 4x. Given a sane output mode it plays essentially at realtime.

**What this localises.** Output resolution changed (4K -> 1440p) and nothing moved. *Source*
resolution changed (4K -> 1080p) and the rate doubled. So the cost is in handling the decoded
**source** frame — decode plus the `drm-copy` transfer — not in scanout. Together with the
bitrate control (12.5x fewer bits, 6% faster), the constraint is now pinned to *source pixels per
second*, which is the one quantity 4K30 fixes and no encoder setting can change.

**Status of the mpv verdict.** Much firmer than at 21:40. Not the sink, not the capture card, not
the bitrate, not the output mode. mpv reaches realtime at 1080p and roughly half realtime at 4K,
on every configuration tried. Still open: **Pi 5 is untested** (different silicon, and the
zero-copy path may simply work there), and mpv's option space is not exhausted.

### 2026-08-13 22:15 — Insta360 X4 rejected as a capture instrument

Settles a SETUP-phase open question (2026-08-12 17:05 entry) — negatively, on three independent
grounds, any one of which is sufficient.

1. **No frame-rate advantage, which was the entire motivation.** In UVC webcam mode the X4 offers
   only `1920x1080@30` and `2880x1440@30`. Its high-frame-rate modes are internal-recording only.
   The premise that it beats the Cam Link's 30 fps ceiling is simply false.
2. **360 equirectangular projection.** The card occupies about a fifth of the frame and is visibly
   barrel-warped, so barcode cells are not linearly spaced across the bar.
3. **The harness's geometry assumes the frame *is* the card.** `BAR_*_FRAC` are fractions of the
   frame, which holds for direct HDMI capture and fails for *any* camera pointed at *any* screen,
   independent of optics. The fixed crop lands on the wrong region entirely.

**The predicted failure mode did not occur, and that is a design win.** The 17:05 entry feared
dewarping would yield "*confidently wrong* indices rather than errors". It did not:
`capture.mjs` returned **`null` on all 60 frames**, because `decodeFrame` checks its white/black
sync cells (`white - black < SYNC_MIN_DELTA`) before trusting anything. That guard is exactly what
stands between a camera experiment and a silently corrupt dataset.

Using any camera would require corner detection plus perspective rectification ahead of the
decoder. Not worth building while the Cam Link path works.

**Hazard found in passing:** plugging the X4 in made it `avfoundation` device **0**, displacing the
Cam Link to **1**. The documented `--source avfoundation:0` therefore silently re-targets whenever
a camera is attached. `scripts/check-capture-link.sh` is unaffected (it matches by name), but the
capture invocation is not.

### 2026-08-13 23:00 — REVERSED: mpv **does** reach realtime at 4K. It was the wrong interop.

**The 21:40 conclusion was wrong.** mpv is not incapable of 4K30 on a Pi 4; I had never tried
the interop that works. Correcting it in full, because it inverts the milestone verdict.

**Root cause, and it explains every earlier number.** `rpi-hevc-dec` can *only* output NV12 in
Broadcom's 128-byte-column **SAND** tiling (`NV12_128C8`, `DRM_FORMAT_MOD_BROADCOM_SAND128`) —
verified directly: forcing `video/x-raw,format=NV12` on the decoder fails with `not-negotiated`.
The Pi 4's HVS **scans SAND out natively**, confirmed on this machine:

```
NV12:  BROADCOM_SAND128(0x700000000000004) BROADCOM_SAND64 BROADCOM_SAND256 LINEAR
```

So zero-copy needs **no conversion at all** — but *only* on the direct-to-KMS-plane path. Every
other path pays a detile cost: `drm-copy` detiles on the CPU (14.3 fps), and GL import makes v3d
sample SAND at 4K (5 fps). That is the entire story of tonight's earlier numbers.

**The fix is one flag.** mpv has *two* DRM_PRIME interops. `drmprime` imports into GL; the
`drmprime-overlay` interop puts the frame on a KMS plane and skips GL for video entirely.

| mpv configuration | ratio | fps | drops |
|---|---|---|---|
| `--vo=drm --hwdec=drm` (probe.sh's old default) | — | ~5 | **software decode, silently** |
| `--vo=drm --hwdec=drm-copy` | 0.476 | 14.3 | 0 |
| `--gpu-hwdec-interop=drmprime-overlay` | 0.947 | 28.4 | 0 |
| **+ `--video-sync=display-resample`** | **0.969** | **29.1** | **0** |

Verified out-of-band, not just by mpv's own clock — 317 captured frames, **317 decoded, 0
nulls, 89 of 90 distinct indices**, steps `+1`x265 / `+2`x40 / `0`x11. The `+2` and `0` steps are
the Cam Link's ~27 fps sampling of a 30 fps display, already characterised. The residual 3 % on
the ratio is most likely `measure-rate.sh`'s own 0.2 s polling overhead.

**Candidate M1 deliverable — the argv** (now `probe.sh`'s default; `--no-overlay` restores the
old behaviour for A/B):

```
mpv --vo=gpu --hwdec=drm --gpu-context=drm --gpu-api=opengl \
    --gpu-hwdec-interop=drmprime-overlay \
    --drm-draw-plane=overlay --drm-drmprime-video-plane=primary \
    --video-sync=display-resample \
    --fullscreen --no-osc --no-input-default-bindings --no-terminal \
    --loop-file=inf <asset>
```

**Other players measured on the same asset and rig:**

| Player | Rate | Notes |
|---|---|---|
| `ffmpeg -hwaccel drm -f vout_drm` | **1.92x** | Fastest by far. jc-kynesim's drmu plane path. Verified on screen: 318/318 decoded, 87/90 distinct. But its author calls it "a development test device, not production-grade" |
| mpv + `drmprime-overlay` | 0.969x | **Best for dex** — M2's target player, and `--loop-file=inf` is a real looping mode |
| GStreamer `v4l2slh265dec ! glupload ! glimagesink` | 0.97x | Works, but GL import — not zero-copy, no headroom |
| VLC `--vout drm_vout` | 0.91x | Logged `Failed to set atomic cap` and fell off the atomic path |
| GStreamer `... ! kmssink` | **fails** | See below |
| GStreamer decode only (`fakesink`) | 1.90x | The stateless decoder alone beats ffmpeg's 1.36x |

**GStreamer `kmssink` is a known upstream gap, not a misconfiguration.** It fails with
`gst_kms_allocator_add_fb: Failed to bind to framebuffer: Numerical result out of range` —
`drmModeAddFB2` rejecting the SAND modifier — then falls back to dumb-buffer allocation and OOMs
at 4K. A Raspberry Pi engineer (6by9) states this directly on the Pi forums about this exact
trixie output format: *"kmssink needs further work to support it when using DMABuf."*
`v4l2convert` cannot help either: it only accepts `video/x-raw(memory:DMABuf), format=DMA_DRM`
and rejects the decoder's caps outright.

**Escalation ladder outcome:** step 1 (pivid) turned out to be unnecessary — its build was still
running when mpv was fixed, and is now moot for M1. Step 2 (GStreamer) is blocked upstream. Step
3 (custom C player) is not needed. Ladder can be stood down.

### 2026-08-13 23:05 — Blocker for the seam measurement: the barcode moved under the experiment

`KTE/dex@835d724` ("move the barcode into the card's label bar"), committed by the parallel
asset-polishing session **while this bench session was running**, changes the barcode geometry:

| | asset on the Pi | `lib/barcode.mjs` now |
|---|---|---|
| x | 1020 | 941 |
| width | 1800 | 1098 |

So the decoder and the bench asset have drifted apart, and **every `capture.mjs` run against the
old asset returns `null`**. All the index streams above were decoded with the *old* geometry,
supplied manually, to keep tonight's playback results valid.

**It failed loudly, which is the design working.** `decodeFrame` checks its white/black sync
cells (`white - black < SYNC_MIN_DELTA`) before trusting anything; with the crop off by roughly
one cell it read `white=150, black=255` and refused. The README's warning — *"a burner and
decoder that drifted apart would fail silently"* — is now the one hazard this harness provably
does **not** have.

**Required before any seam run:** rebuild the bench asset with current `lib/barcode.mjs`, then
re-verify `capture.mjs --source <file>` returns 0..89. No seam number is meaningful until then.

### 2026-08-13 23:20 — The 4K frame-rate ceiling on Pi 4, measured. **4K30 is the target.**

Prompted by a goal that named 4K40 (later confirmed a typo). Worth answering anyway, because it
bounds dex's spec rather than leaving the frame rate an assumption.

**Decode ceiling — measured without any display, so it is the silicon's limit:**

| Asset | `ffmpeg -hwaccel drm` speed | Implied 4K HEVC decode rate |
|---|---|---|
| 4K**30** clouds (39.3 Mbps) | 1.36x | ~41 fps |
| 4K**60** card | 0.753x | ~45 fps |

So the Pi 4 decodes roughly **41–45 fps of 4K HEVC**. That is the hard bound, before any
display path cost.

**Consequences:**

- **4K30 — comfortable.** 0.969x with zero drops through mpv's overlay path, ~35 % decode
  headroom. This is the right target.
- **4K40 — marginal at best, and untestable here.** It would need essentially all the decode
  budget with nothing left for presentation, *and* a >=40 Hz 4K mode, which no sink on this
  bench provides.
- **4K60 — out of reach.** Decode alone runs at 0.753x. Confirmed end to end: playing the 4K60
  asset through the working mpv path reports `ratio=0.976` but **83 dropped frames** — it holds
  the clock by discarding frames, which is exactly the failure the "correct playback" bar (§4.4
  A2) exists to catch. A drop-free reading of the ratio alone would have called this a pass.

**Display-side limit, independent of decode.** With the Cam Link attached, no 4K mode above
30 Hz exists at all: its EDID is HDMI **1.4** and caps at 2160p30 (297 MHz). Forcing
`video=HDMI-A-1:3840x2160@60` *and* `hdmi_enable_4kp60=1` did **not** take — TMDS stayed at
297 MHz, because the driver will not synthesise a mode the sink does not advertise. 4K60 needs
594 MHz and an HDMI 2.0 sink. The `--untimed` runs both converging on ~30 fps are therefore
measuring the 30 Hz vsync, not the pipeline.

**Decision (Max, 2026-08-13): dex targets 4K30.** Recorded so the frame rate stops being an open
assumption in M3 (transcode-on-ingest) and M4 (exhibition format) — ingest can normalise to 30 fps
knowing 60 was ruled out by measurement, not by guesswork.

---

## Failed Attempts

| # | What Was Tried | Why It Failed | Lesson |
|---|---------------|---------------|--------|
| 1 | Diagnosed the all-zero 4K capture as an **HDMI physical-layer failure at 297 MHz** — cable, micro-HDMI adapter, or the Cam Link's receiver | Wrong layer entirely. The real fault was USB 2.0 enumeration. Every measurement in the diagnosis was *correct* — the Pi did send exactly what the EDID advertised, 1080p at 148.5 MHz did work, and all three CEA 4K modes genuinely do run at 297 MHz — and none of it was *relevant* | Measuring the rawest layer first (raw bytes over ConsoleLink) was right and worked. The error was concluding from one link in the chain without enumerating the others. **The USB link speed was one `ioreg` call away the entire time.** Check every hop before committing to a hypothesis about any hop |
| 2 | Tried to lower the HDMI bandwidth by forcing 2160p24 / 2160p25 | Not a syntax failure — the modes applied correctly (`type: userdef` in `modetest`). CEA-861 gives 2160p24/25/30 a **common 297 MHz clock**, varying only horizontal blanking (htotal 4400 / 5280 / 5500) | "Drop the frame rate to fit the link" is a reflex that does **nothing** at 4K. Read the actual mode table before assuming a knob exists |
| 3 | Reported a mangled `cfg80211.ieee80211_regdom=CHdtparam=audio=on` in `cmdline.txt` | Artifact of reading `cmdline.txt` (no trailing newline) and `config.txt` in one batched command | Do not diagnose from concatenated command output. Retracted before anyone "fixed" a file that was always correct |
| 4 | Concluded at 21:40 that **mpv cannot reach realtime at 4K** on Pi 4, after sweeping `--vo` x `--hwdec` | The sweep covered the wrong axis. mpv has *two* DRM_PRIME interops and every configuration tried used the GL one (`drmprime`); the KMS-plane one (`drmprime-overlay`) doubles the rate to 0.969x. Every measurement was right; the option space was mis-mapped | Sweeping many values of the parameters you *know about* feels exhaustive and is not. Before declaring a tool incapable, enumerate its extension points (`--gpu-hwdec-interop=help` would have shown this in one command) rather than its settings |
| 5 | Chased the all-null capture as a *display* fault under `vout_drm` — suspected modeset, scaling, offset | The display was perfect; `lib/barcode.mjs` had been changed by a concurrent session mid-run, so the decoder was looking 79 px left of a bar that had also narrowed by 702 px | When a measurement breaks after being correct, check whether the *instrument* changed before re-examining the subject. `git log` on the harness is a diagnostic step |

## Findings

<!-- REQUIRED in ANALYZE. Tables for quantitative data. Quote specifics, not vibes. -->

_Not yet in ANALYZE phase._

## Verdict

<!-- REQUIRED in VERDICT phase. -->

**Outcome:** _pending_
**Confidence:** _pending_
**Evidence:** _pending_

**Next Action:** _pending_
