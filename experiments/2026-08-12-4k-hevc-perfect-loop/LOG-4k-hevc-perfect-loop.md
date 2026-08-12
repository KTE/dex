---
phase: FRAME
mode: rigorous
started: 2026-08-12
timebox: open-ended
slug: 4k-hevc-perfect-loop
tags: [dex, video, hevc, 4k, raspberry-pi, mpv, seamless-loop]
verdict: null
confidence: null
---

# Lab Notes: 4K HEVC Perfect Loop — does mpv on Pi OS Trixie wrap without a seam?

**Full design:** [PLAN.md](PLAN.md) — harness architecture, asset spec, measurement method,
test matrix, escalation ladder.

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

There exists an `mpv` invocation on Raspberry Pi OS Trixie that loops a 4K30 HEVC file with **no
seam** — defined as a wrap-point anomaly rate statistically indistinguishable from the same run's
mid-loop capture noise floor — on **both Pi 5 and Pi 4**, sustained without drift over 24 hours.

## Success Criteria

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

For **every** config in the matrix, on **both** boards: wrap-point anomaly rate exceeds the
mid-loop baseline by a statistically significant margin (p < 0.01 over >=500 wraps).

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

**If confirmed:** freeze the argv verbatim into `PLAN.md` findings. Proceed to milestone 2 — fork
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

<!-- REQUIRED before advancing to RUN. Fill at SETUP: OS image + version, mpv version, ffmpeg
     version and whether it is the +rpt2 build, kernel, board revisions, display EDID/mode,
     Cam Link firmware, cable. -->

_To be filled during SETUP._

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
masters (see PLAN.md §3).

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
PLAN.md §4.3 as Controls 2 and 3.

---

## Failed Attempts

| # | What Was Tried | Why It Failed | Lesson |
|---|---------------|---------------|--------|

_(empty — nothing run yet)_

## Findings

<!-- REQUIRED in ANALYZE. Tables for quantitative data. Quote specifics, not vibes. -->

_Not yet in ANALYZE phase._

## Verdict

<!-- REQUIRED in VERDICT phase. -->

**Outcome:** _pending_
**Confidence:** _pending_
**Evidence:** _pending_

**Next Action:** _pending_
