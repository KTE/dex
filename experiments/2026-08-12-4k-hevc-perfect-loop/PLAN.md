# Experiment Plan — 4K HEVC Perfect Loop

**Experiment:** `4k-hevc-perfect-loop` · **Started:** 2026-08-12 · **Mode:** rigorous
**Running record:** [LOG-4k-hevc-perfect-loop.md](LOG-4k-hevc-perfect-loop.md)
**Research base:** `home-workspace/projects/dex/research/2026-07-26-4k-true-looping/`
(DOSSIER-1, DOSSIER-2, SCOPE, streams 1-12)

---

## 1. The question, the deliverable, the scope

**Question.** Is there an `mpv` invocation on Raspberry Pi OS Trixie that loops a 4K HEVC file with
no seam, on Pi 5 and Pi 4, sustained over 24 hours?

**Deliverable, in priority order:**

1. **An argv.** A literal `mpv` command line that loops perfectly. This is the artifact milestone 2
   consumes — `mpv.py`'s `play()` is a `subprocess.Popen` of exactly this list. A prose description
   of "what worked" does not satisfy this.
2. **Evidence.** A measured anomaly rate across hundreds of wraps, with controls.
3. **A decision.** Pass -> milestone 2. Fail -> escalate per the fixed ladder (§6), with a
   measurement in hand rather than starting that argument from zero.

**Out of scope, deliberately.** No image building (`sdm`, `rpi-image-gen`, pi-gen), no FAT data
partition, no read-only root, no `pi_video_looper` integration, no transcoding, no audio. The Pi
runs **stock Trixie Lite** flashed with Raspberry Pi Imager plus `apt install mpv`.

The reasoning is asymmetric and worth stating: if the probe **fails**, every hour spent on
packaging was wasted. If it **passes**, packaging is a known, boring problem dex has already solved
once. There is no scenario where doing packaging first pays.

**One non-goal that costs nothing to protect:** audio. Not tested here, but recorded as a constraint
on *backend selection* — do not choose a structurally silent-only player. `mpv` satisfies this for
free. (This is also why `hello_video` cannot be the long-term answer even setting aside buster.)

## 2. Harness architecture

Lives in this directory. Seven units with hard boundaries:

| Unit | Runs on | Does | Depends on |
|---|---|---|---|
| `make-master.sh` | Mac | Generates the procedural lossless master at any res/fps/duration | ffmpeg |
| `add-barcode.sh` | Mac | Burns the binary frame-index row onto any master | ffmpeg |
| `encode-variants.sh` | Mac | Master -> encode matrix + player sidecars | ffmpeg |
| `probe.sh` | Pi | Runs one named mpv config, captures mpv's in-band stats | mpv |
| `capture.mjs` | Mac | Cam Link -> ffmpeg -> decode barcode -> frame-index log | Node, ffmpeg |
| `analyze.mjs` | Mac | Frame-index log -> wrap-by-wrap anomaly report + verdict | Node |
| `soak.sh` | Mac | Long-run wrapper; saves video clips only around anomalies | the above |

**Two boundaries carry the design:**

- **`capture.mjs` decides nothing.** It converts pixels into a stream of integers and stops.
- **`analyze.mjs` touches no hardware.** It takes a log file and emits a verdict.

That split buys three things: any historical capture can be replayed against a revised analysis;
the analyzer is unit-testable against synthetic logs with **no hardware at all**; and a disagreement
between the two signals is localisable rather than mysterious.

**Language.** `.mjs` with `// @ts-check` and JSDoc, **zero runtime dependencies** — so the harness
runs on the bench Pi, the Mac, or a borrowed laptop with nothing but Node installed. At 23:00 on a
bench, `pnpm install` should not be between you and an answer.

**Storage.** The analyzer logs only the decoded frame index per captured frame — a few bytes each.
24 h at 30 fps is 2.6 M integers, a trivially-sized log. **So the soak is continuously measured
rather than sampled**, which is a far stronger 24 h result than "it was still running in the
morning." Video is retained only in short clips around detected anomalies.

## 3. Test assets

`packages/example-content` is already a **multi-player test kit** built in 2024: for each variant it
emits the encode plus player-specific sidecars — `.mp4.h264` elementary stream for `hello_video`, a
`.json` pivid timeline, an `.html` `<video loop>` for Cog/WPE — at 1s / 2s / 3s durations. The A/B
rig for this comparison already exists. This experiment **extends it rather than replacing it**.

### 3.1 Two masters, one on the critical path

| Master | Produced by | Purpose | Critical path? |
|---|---|---|---|
| **Procedural 4K** | `make-master.sh` (this repo) | **The automated experiment's input.** Smooth motion, matched wrap frames, high-frequency detail so the decoder is honestly loaded. Regenerable at any res/fps/duration by changing an argument. | **Yes** |
| **AE 4K render** | Max, from `test-cards.aep` + `AltekaKard-4K.png` | Eyeball checks and demos. Its rotating elements are tuned for *human* perception of loop quality — which is a different instrument than the analyzer, and a legitimate one. | **No** |

The automated pipeline never waits on the AE render. They cannot drift, because **the analyzer
depends only on the barcode, never on the content** — `add-barcode.sh` applies identically to both.

### 3.2 The frame barcode

A row of 16 high-contrast blocks across the top of the frame encoding the frame index in binary,
plus a fixed 2-block sync pattern so the decoder can locate the row and set its threshold after any
scaling the capture path applies. 16 bits = 65,536 frames ~= 36 min at 30 fps.

Read by sampling block centres — **no OCR**. A burned-in numeral would need character recognition,
which is fragile under rescaling and compression; large high-contrast blocks survive both, and the
decision is a luma threshold, so chroma subsampling is irrelevant.

### 3.3 Variants

| Variant | Purpose |
|---|---|
| `4k30-1s` | **Primary.** The case dex exists for, and the fastest to statistical confidence — 3,600 wraps/hour |
| `4k30-10s` | Longer-GOP behaviour; guards against a result that only holds for tiny files |
| `4k60-10s` | Stretch check on Pi 5, measured via the weaker 1080p60 capture path |
| `1080p30-h264.h264` | **Positive control** (§4.3) — same source content as a raw H.264 elementary stream for `hello_video` on the dexOS card |

Encoding: HEVC, closed GOP, IDR at frame 0, keyint = 1 s, silent, ~40 Mbps at 4K30 — comfortably
under the Pi 4 ~80 Mbps HEVC ceiling.

## 4. Measurement

Two independent signals, deliberately **not** merged into one number. They can disagree, and a
disagreement is itself a finding.

### 4.1 In-band (free, always available)

`mpv --dump-stats` plus `vo-delayed-frame-count` / `frame-drop-count`. Costs nothing, and crucially
it runs during the **24 h soak where no capture is attached**. This is mpv's own account of whether
it believes it dropped anything.

### 4.2 Out-of-band (the real witness)

Cam Link 4K -> ffmpeg -> `capture.mjs` -> frame-index stream -> `analyze.mjs`:

| Observation | Means |
|---|---|
| index increments by 1 | normal |
| index repeats | held frame |
| index skips forward | dropped frame |
| index resets to 0 | **wrap point** |
| wrap spans > 1 frame, or a black/undecodable frame appears at wrap | candidate seam |

**Cam Link properties to verify at the bench, because both bite:** it *accepts* 4K60 input but
**records 4K at 30 fps**; and it has **no HDMI passthrough**, so the Pi feeds the capture card, not
the projector, unless a splitter is added. Capture and the projector A/B are therefore sequential
activities.

### 4.3 The controls — why this method is trustworthy

USB capture is not frame-locked to the Pi, so it drops and duplicates frames on its own. Without
controls, that noise makes any result meaningless. Three controls, each answering a different
failure of the instrument:

**Control 1 — the noise floor (is the capture lying?).** Capture artifacts land *uniformly at
random*; a real seam lands *at the wrap, every time*. So the **mid-loop anomaly rate is the noise
floor**, measured simultaneously, on the same run, by the same instrument. The verdict is therefore
a comparison, never an absolute:

> **Pass = wrap-point anomaly rate statistically indistinguishable from the mid-loop baseline,
> over >=500 wraps.**

Made explicit so "indistinguishable" is not left to judgement: a **two-proportion test** comparing
`anomalies at wrap transitions / total wrap transitions` against
`anomalies at non-wrap transitions / total non-wrap transitions`, on the same run. **Pass = p >= 0.01**
(no detectable difference). **Fail = p < 0.01** with the wrap rate the higher of the two. A wrap rate
*lower* than baseline is not a pass — it means the analyzer is misclassifying wraps, and the run is
void.

With the 1-second asset that is under 10 minutes of capture — which is the whole payoff of making
the 1 s variant first-class rather than a follow-up.

**Control 2 — synthetic defect injection (is the analyzer blind?).** `analyze.mjs` is validated
against generated logs with known defects planted: a held frame at wrap, a dropped frame at wrap, a
black frame. If it cannot catch a defect you planted, "no anomalies detected" is not evidence of
anything.

**Control 3 — `hello_video` as positive control (does the analyzer invent defects?).** The barcode
is burned into the 1080p H.264 master too, so the **dexOS card runs through the identical rig**. It
is the only *proven* seamless loop in existence here. If the analyzer reports it at baseline, the
end-to-end method is validated in hardware. If the analyzer reports a seam on a known-good loop,
the rig is wrong and every other number it produced is void.

Controls 2 and 3 are not redundant: injection proves sensitivity, `hello_video` proves specificity.
A method with only one of them can be confidently wrong in one direction.

### 4.4 Full pass bar

- Wrap rate at baseline over >=500 wraps, 4K30, Pi 5 — **and** the same config on Pi 4
- 24 h soak with no drift in in-band counters
- All three controls satisfied
- A/B against the dexOS card on the same display shows no difference to the eye
- The argv is written down verbatim

## 5. Test matrix, order, and the stop condition

**Loop mechanisms** (DOSSIER-2 §5):

1. `--loop-file=inf`
2. `--ab-loop-a=0 --ab-loop-b=<dur-eps>` — seeks before EOF, never touching EOF handling
3. `--keep-open=yes` + scripted seek over the IPC socket

crossed with VO/hwdec combinations (`--vo=drm` vs `--vo=gpu --gpu-context=drm`; `--hwdec=drm` /
`v4l2request`). The exact combinations are **enumerated at the bench** against what Trixie's `+rpt2`
ffmpeg actually exposes, not guessed here.

**Order is chosen to reach an answer fastest, not to be complete:**

1. Pi 5 + `4k30-1s` + mechanism 1. If it passes, the argv exists within the hour.
2. Expand the matrix only as things fail.
3. Re-test on Pi 4 **only once something passes on Pi 5** — testing two boards before one config
   works multiplies unknowns.
4. `4k30-10s`, then the 4K60 stretch check.
5. 24 h soak on the winner.

**Stop condition, pre-committed:** 2 bench days without any config reaching baseline on either
board -> stop, declare REFUTED, escalate. Pre-committing this is the point; in the moment it will
feel like one more config might do it, and that feeling is exactly what turned this into a
12-year-old open question.

## 6. Escalation ladder (fixed, no re-litigation)

1. **pivid on Pi 4** — cheapest, because `example-content` already ships pivid `.json` timelines.
   Settles `status.md` open question #5 either way. Caveats: dormant since 2024-04-10, Conan
   bitrot, Pi 4 only. Max authored its final commit (PR #15), so the build system is not foreign.
2. **GStreamer segment-seek** (`v4l2slh265dec` + kmssink). Caveat: Trixie kmssink is not yet
   zero-copy for HEVC.
3. **Custom C player** — ~2k lines against Trixie system libav, 10-15 person-days, DOSSIER-2 §5
   Rank 1, with its own 5-day tracer stop condition (§6.4 there).

**Not permitted on refutation:** more mpv configurations.

## 7. Roadmap — milestones 2-4 (recorded, not specced)

Each gets its own plan when milestone 1 passes.

**M2 — `pi_video_looper` integration.** Fork (not quilt — a new backend file patches nothing
upstream, so quilt would be ceremony with no payoff). Write `Adafruit_Video_Looper/mpv.py` exposing
`create_player()` plus a class with `play` / `is_playing` / `stop` / `supported_extensions`; the
loader at `video_looper.py:137` does `importlib.import_module('.' + module, 'Adafruit_Video_Looper')`,
so a backend is genuinely one file. `hello_video.py` is 80 lines and is entirely a
`subprocess.Popen` wrapper — which is why the milestone-1 argv is the deliverable. Set
`video_player = mpv` in the `.ini`, repoint dex's submodule. Upstream is **GPLv2**, so the fork is
permitted and stays GPLv2.

Then: stabilise -> open an upstream issue linking the fork, asking whether an mpv PR would be
welcome -> PR **only on positive signal**. Shipping never blocks on a reply. Re-run `analyze.mjs`
against the integrated backend as a regression test — the same instrument proves the integration
did not break the loop.

**M3 — transcode on ingest.** Extends `usb_drive_copymode`: any file without a playable sibling gets
ffmpeg-converted onto `/dexdata` during copy. This is `status.md` next-action #1, unchanged since
2024, except the target is now HEVC rather than raw `.h264`. Worth stating plainly: this is **not
merely convenience**. It is where GOP structure and keyframe placement get normalised — and those
determine whether a seamless loop is achievable at all. M3 is where loop-ability is *enforced*.

**M4 — dex exhibition format.** Playlist definition with per-video timing and transforms
(rotate / mirror). Cheap on mpv (`--video-rotate`, `--vf=hflip`), cheaper on DRM planes. The
existing `.json` sidecar convention in `example-content` is a plausible starting shape.

## 8. Known risks

| Risk | Mitigation |
|---|---|
| Cam Link caps 4K capture at 30 fps | 4K30 is the measured case by design; 4K60 is an explicitly weaker stretch check, labelled as such |
| No HDMI passthrough on Cam Link | Capture and projector A/B run sequentially; add a splitter only if simultaneity turns out to matter |
| Capture not frame-locked -> false anomalies | The mid-loop noise floor is the control (§4.3) |
| Analyzer blind to real defects | Synthetic injection (Control 2) |
| Analyzer inventing defects | `hello_video` positive control (Control 3) |
| Trixie mpv lacks the expected hwdec/vo combos | Enumerated at the bench, not assumed; DOSSIER-2 [S4] says `+rpt2` carries the Pi 5 HEVC patches |
| Open-ended grind on mpv configs | 2-bench-day early stop, pre-committed with a fixed escalation ladder |
| dexOS card unavailable or dead | Controls 1 and 2 still stand; Control 3 and the eyeball A/B are lost. Degrades the result, does not void it. |
