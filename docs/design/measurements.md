# Measurement record

This page records every number the dexd documentation relies on, with the conditions it was taken under and how it was established, for a reader checking what a figure rests on. Other pages state a number and link here.

Unless a line says otherwise, the hardware is a Raspberry Pi 4 Model B running Debian trixie and the video is HEVC in a raw Annex-B stream. A number with no other label is measured; *derived*, *assumed* and *not tested* mark the rest.

## The instrument

The measurement rests on a test card carrying its frame index as a [barcode](../glossary.md#barcode) burned into every frame, beside a grey ramp, colour wheels, a checkerboard border, resolution wedges and a set of rotating hands. A capture device — an Elgato Cam Link 4K — records the player's HDMI output. One script decodes the barcode into a stream of integers; a second turns that stream into a verdict and touches no hardware, so the analysis runs against synthetic input and replays any recorded run. The analysis tooling is not part of the dexd package.

The analysis reads the decoded index stream one consecutive pair at a time:

| Index behaviour | Reading |
|---|---|
| steps by 1 | a normal transition |
| repeats | a held frame |
| skips forward | a dropped frame |
| resets to 0 | a loop point |
| undecodable | a decode failure, counted against the side it falls on |

The verdict is a comparison, never an absolute count: the capture device is not frame-locked to the player, so it drops and repeats frames on its own, spread evenly through the run, while a defect at the loop point concentrates there. The rate away from the loop point is the [noise floor](../glossary.md#noise-floor), measured on the same run by the same instrument.

### Verdicts

A [two-proportion test](../glossary.md#two-proportion-test) compares the anomaly rate at loop-point transitions with the rate at all other transitions in the same run.

| Verdict | Condition |
|---|---|
| PASS | p ≥ 0.01 — the loop-point rate is indistinguishable from the noise floor |
| FAIL | p < 0.01 with the loop-point rate higher |
| VOID | p < 0.01 with the loop-point rate lower, meaning loop points are misclassified — check the loop length |
| INSUFFICIENT | fewer than 500 loop-point transitions captured |

Sensitivity scales with the noise floor: against a perfectly clean sample away from the loop point, one anomaly in 600 loop points gives z = 5.4, p ≈ 7×10⁻⁸, while the same defect passes against a realistic 1% floor. A cleaner capture path makes the test stricter, so improving one mid-experiment invalidates comparison with earlier verdicts.

**Note:** at a 1 s loop, one defect in 600 loop points is a visible stutter every ten minutes. The 1 s 4K30 test video runs 3,600 loops per hour, so 500 loop points take under ten minutes of capture (derived).

### Two channels

The measurement records two signals: mpv's own reporting, which needs no capture device, and the capture chain. They share no code, and a disagreement between them is itself a finding. One 4K30 run read as follows.

| Signal | Channel | Reading |
|---|---|---|
| mpv's playback position against the wall clock, sampled by a script | mpv | [realtime rate](../glossary.md#realtime-rate) 0.476 — 14.3 fps of a required 30 |
| mpv's frame counters | mpv | dropped 0, decoder-dropped 0, late 0 |
| the decoded barcode | capture | advanced about 14 indices per second |

mpv's counters alone cannot establish correct playback: at 14.3 fps it presented every frame, far too slowly, and every counter above read 0. Only playback time against the wall clock catches that.

### Pass criteria

The analysis checks correct playback first; a failure there makes the run VOID for loop-point purposes rather than a loop-point failure.

| Clause | How it is checked |
|---|---|
| Realtime rate | mpv's playback position against the wall clock; ratio ≥ 0.98 |
| Full frame rate at the display | the captured index steps by 1, agreeing with the realtime-rate clause |
| Colour | the grey ramp and the colour wheels against the source |
| Geometry | the checkerboard border complete on all four edges, and the resolution wedges — converging line patterns that go grey where detail is lost |
| Native resolution | TMDS character rate and framebuffer size, with the wedges legible |

A person checks the last three clauses by eye. A configuration that cannot reach correct playback fails outright.

**Note:** the card's own frame counter resets at the loop point, a large visible discontinuity at the same place as an accidental one. For a comparison by eye, watch the rotating hands.

### Controls

The noise floor above is the first control, measured on every run. Two more run first, and both must pass before any hardware number is taken.

| Control | What it establishes | Result |
|---|---|---|
| Planted defects | the analysis catches what it should, and only that | 1% of frames duplicated at random positions → PASS; one held frame in 600 loops → PASS; a held final frame at every loop → FAIL; a deterministic mid-loop anomaly → VOID |
| Positive control | the analysis invents no defects on a known-good loop | the legacy hello_video player on its own raw H.264 test video passes; if not, measurement stops |

A fourth check needs no hardware: one verified single-loop capture, concatenated 600 times as a perfect player would put it on the wire, analyses as PASS.

### Test videos

The pipeline renders the card, burns in the barcode and encodes the result as HEVC with a closed GOP — keyframe interval equal to the frame rate, scene-cut detection off, open GOP off, IDR at frame 0, silent. It then decodes each encode again and reads the barcode back: a barcode that failed without a message would make every measurement from that file wrong.

Each card yields two variants: a clean one at 3–9 Mbps, isolating the loop point, and a cloud-textured one at 20–39 Mbps, loading the decoder. Flat colour and static geometry compress to roughly a tenth of what real video produces — 3.1 Mbps at 1080p against a 20 Mbps target, 8.5 Mbps at 4K against a 40 Mbps target. Clean passing and textured failing means decoder load.

Test videos exist at 1, 2 and 3 s, at 1080p, 4K30 and 4K60. The 4K reference video is 3840×2160 at 30 fps, 90 frames per loop, 39.7 Mbps, three closed-GOP keyframes, IDR at frame 0, barcode verified 0–89 on every frame after encoding.

The card's motion returns to its starting position at the loop point. The rotating hands advance 6° per frame, one full revolution every 60 frames, and stand at 354° on the last frame of that revolution, so the step back to 0° is +6° like every other. A 4K30 file is decimated from a 60 fps render, exact for synthetic content with no motion blur; the pipeline refuses a non-integer decimation factor.

## Playback-path throughput

Conditions: Raspberry Pi 4, 3840×2160 at 30 fps, the ~39 Mbps cloud-textured test video, output to the capture device at 3840×2160 in RGB 4:4:4 — one full colour sample per pixel — at 8 bits per component, TMDS character rate 297 MHz.

| mpv path | What touches the frame | Reading |
|---|---|---|
| `--vo=drm --hwdec=drm` | nothing decodes in hardware — mpv selects the software decoder | ~5 fps |
| `--gpu-hwdec-interop=drmprime` | the GPU samples the SAND-tiled frame as a texture | ~5 fps |
| `--hwdec=drm-copy` | the CPU detiles SAND into linear NV12 | 14.3 fps (ratio 0.476) |
| `--gpu-hwdec-interop=drmprime-overlay` | nothing — the frame handle goes to a KMS plane | 28.4 fps (ratio 0.947) |
| the same plus `--video-sync=display-resample` | nothing — mpv also paces presentation to the display's measured refresh | 29.1 fps (ratio 0.969), zero drops |

The decoder emits SAND-tiled NV12, which the display block scans out natively only from a KMS plane; every other path detiles first, and detiling costs most of the frame rate. dexd uses the last row's options (see [architecture.md](architecture.md)).

The overlay path runs at 30 fps. Three instruments independent of the capture device agree:

- the kernel's count of display refresh intervals over 10 s reads 29.9993 Hz;
- mpv's estimated display frame rate reads 30.000002, and its `vsync-jitter` property reads 0.000183, a fraction of one refresh interval;
- the HVS underrun counter reads 0, the chip reporting no throttling at a 550 MHz core clock.

The sampling script's 0.2 s polling overhead accounts for the residual 3% in the 0.969 ratio (assumed): 29.1 fps is that script's number, 30 fps the display's. Capture agrees — 317 frames captured, 317 decoded, none undecodable, 89 of 90 distinct indices present.

### Other players

Same hardware, same video, same output mode.

| Player and sink | Throughput | Outcome |
|---|---|---|
| ffmpeg `-f vout_drm` | 1.92× | fastest measured, but stalls on keyframes three times per loop; described upstream as a development test device, not production-grade (see [vout_drm](../glossary.md#vout_drm)) |
| GStreamer, decode only to a null sink | 1.90× | the stateless decoder alone, no display path |
| GStreamer `glimagesink` | 0.97× | works, but imports through the GPU and leaves no headroom |
| VLC `--vout drm_vout` | 0.91× | logs a failure to set the atomic capability and leaves the atomic path |
| GStreamer `kmssink` | fails | cannot bind a SAND dma-buf, falls back to CPU copies and runs out of memory at 4K |
| pivid | not tested | purpose-built for gapless playback, dormant since 2024 |

### Decode ceiling

The decoder alone, with no display attached.

| Content | Throughput | Implied rate (derived) |
|---|---|---|
| 4K30, 39.3 Mbps | 1.36× | ~41 fps |
| 4K at 40 fps | 1.08× | ~43 fps |
| 4K60 | 0.753× | ~45 fps |

The Pi 4 decodes roughly 41–45 fps of 4K HEVC, the bound before any display cost. Through the working display path mpv plays the 4K60 file at a ratio of 0.976 and drops 83 frames — it keeps pace with the clock by discarding frames.

Throughput tracks pixels per second, not the resolution label: 4K30 is 249 [Mpix/s](../glossary.md#mpixs), 1440p60 is 221, 4K60 is 498. About 250 Mpix/s is the practical Pi 4 budget for a full player pipeline (derived); [pi-capability.md](pi-capability.md) sets these beside vendor claims and independent reports.

### Bitrate and sink

Neither the bitrate nor the sink moves the frame rate much; the per-frame 4K copy sets it.

| Change | Reading |
|---|---|
| Bitrate 39.3 → 3.1 Mbps, same 3840×2160 30 fps closed-GOP file | 14.3 → 15.2 fps, a 6% gain |
| The 4K file into the capture device | 14.3 fps |
| The same 4K file into a 2560×1440 monitor | 13.9–14.6 fps, 14–17 drops |
| A 1080p file into that monitor | 28.5 fps, zero drops |

That copy scales with pixels times frame rate, so encoder settings cannot fix a detiling path.

**Note:** forced to 3840×2160, that 1080p file is software-upscaled fourfold and runs slower than the 4K file; the 28.5 fps reading needs an output mode near the source resolution.

### Memory and process cost

| Quantity | Value | Conditions |
|---|---|---|
| Resident memory, dexd | flat at 221 MB over 20 s | 4K30 test video; mpv's read-ahead bounded (`demuxer-readahead-secs=1.0`, `demuxer-max-bytes=64MiB`) |
| Loop payload in memory | 1.3 MB at 1080p, 14.8 MB at 4K | a 3 s test video, read once at startup |
| The same design written in Python | 0.6× realtime | frames held at random points, the signature of a data source not keeping up |
| `while true; do cat loop.265; done \| mpv -` | no held frames, no memory growth over 3.5 h | the shell pipeline the endless stream replaces |

## Loop point

Held frames at the loop point, per looping mechanism. Conditions: Raspberry Pi 4, 4K30 and 1080p60 output, HDMI capture, the 3 s test video.

| Mechanism | Held frame | Per loop | Where |
|---|---|---|---|
| mpv `--loop-file=inf` | 83 ms | 1 | the loop's last frame |
| mpv `--ab-loop-a`/`--ab-loop-b` | 83 ms | 1 | the loop's last frame |
| mpv `--playlist` with `--prefetch-playlist=yes` | 117–133 ms | 1 | the loop's last frame |
| ffmpeg `-stream_loop -1 -f vout_drm` | 67–217 ms | 3 | at keyframes, not at the loop point |
| a file concatenated twelve times, decoded continuously | none | — | — |
| dexd's endless stream | none | — | — |

All three mpv mechanisms re-enter the file — one seeks at end of file, one seeks before it, one opens the next playlist entry — and all three stall. [endless-stream.md](endless-stream.md) describes the stream that replaces them. On a raw .265 file, `--loop-file=inf` freezes on the last frame.

At 1080p60 output the device delivers every frame, so 30 fps content is [oversampled](../glossary.md#oversampling) twofold: every source frame occupies exactly two captures, and a held frame four or more — the ones measured here occupy five. Read `2 ×5497` in the [frame-duration histograms](../glossary.md#frame-duration-histogram) below as 5497 source frames of two captures each.

| Run | Captures per source frame | Held frames |
|---|---|---|
| 61 loops, 11302 captured frames, no decode failures | 2 ×5497, 5 ×60, 4 ×1 | index 89 on 61 of 61 loops |
| 30 s, 10 loops | 2 ×873, 5 ×10 | index 89 on 10 of 10 loops |
| 7 loops, seeking at end of file, and 7 seeking before it | 2 ×582, 5 ×7 each | index 89 on 7 of 7 loops |
| 7 loops, concatenated file, no seek | 2 ×597 | none |
| 8 loops, endless stream | 2 ×749, 1 ×2 | none |

Five captures at 60 Hz is 83.3 ms against 33.3 ms for every other frame, so the last frame stays on screen 50 ms too long and the loop period is 3.050 s against a nominal 3.000 s (derived).

At 4K30 the capture is not oversampled and carries the instrument's ~10% deficit, which raises the noise floor. Over 19 loops the same comparison gives index 89 held ten times with a seeking configuration and no held frames on the endless stream.

The statistical run the verdict rules were written for — 500 or more loop points at 4K30 — has not been run. Three results stand in its place: 19 loops at 4K30, the deterministic oversampled runs above, and the counters from the twenty-five-hour run below.

## Long-running tests

### Twenty-five-hour run

Conditions: a Raspberry Pi 4 Model B Rev 1.1 on Debian 13 trixie, kernel 6.18.34+rpt-rpi-v8. dexd was installed from its .deb, started by systemd 11 s after boot. The asset: 3840×2160 at 30000/1001, 25.3 Mbps, 39.015 s per loop, frame rate from the sidecar. The output: a zero-copy KMS plane into the capture device, 4K30 forced display mode. Total run 25 h 30 min 03 s.

| Quantity | Result |
|---|---|
| Loop count | 2355 |
| Dropped frames | 0 on every heartbeat reporting a number |
| Late frames | 0 throughout |
| Service restarts | 0; start timestamp at boot, one boot record for the window (no reboot) |
| Resident memory | 322244 kB on all 1416 samples, minimum equal to maximum — a 25.3 Mbps 4K30 asset, where the 221 MB above is the lighter test video |
| Chip temperature | 40.8–45.2 °C from the heartbeats, the three highest inside the first half hour |
| Throttle bits | never set |
| Processor use | 24.0–24.2%; the processor clock at its 700 MHz floor on 1350 of 1416 samples |
| Heartbeat continuity | 154 lines, largest gap 601 s |

Loop count times loop length reconciles with uptime to 100.08% — 2355 × 39.015 s = 91880 s against 91803 s — a stall check needing no capture.

The media clock finished 90 s ahead of the wall clock: the final heartbeat reads uptime 91803 s and position 91892.9 s, a ratio of 1.00098 — the 1001/1000 factor between 29.97 and 30 Hz, to within measurement. The video is tagged 30000/1001 against a nominal integer 30 Hz forced display mode, so presenting one frame per display refresh runs 29.97 content 0.1% fast. That accumulates as slow clock skew, which is why both drop counters read 0 while the clocks diverge.

Nine conditions were set for the run, and seven hold: no restarts, one boot record, unbroken heartbeats, no dropped frames, no late frames, 2217 loops at 24 h against a threshold of 2190, and `pos-age=0s` on every line. Two do not:

| Criterion | Status |
|---|---|
| Resident memory: slope below 0.5 MB/h from t+1h to t+24h, growth under 25 MB | Telemetry started 1 h 56 min after the service did, covering 23 h 37 min — 92.6%, unbroken, largest sample gap 61 s — and the value is bit-identical across all 1416 samples, showing no growth. The condition's telemetry clause voids the reading, because the gap exceeds ten minutes: the evidence shows no growth and the condition is not met as written. |
| A human watching at start, middle and end | No observation recorded. |

The run departs from the criteria in two ways, and leaves one measurement out:

| Departure | What it means |
|---|---|
| Asset and sink | It played the 4K30 video into the capture device; the criteria named a 45 Mbps 1440p60 video on a 2560×1440 monitor. 4K30 is 248.6 Mpix/s against 221.0, so the run subsumes the lighter one on pixel rate by 12.5% — but not on bitstream load, at 25.3 Mbps against a 45 Mbps cap. |
| Build identity | The criteria pinned a build whose `--version` reported `0.1.0 (nogit)`; the build that ran is three commits later and self-reports its commit. |
| Capture omitted | The capture device cannot resolve a held frame at 4K, so every number above is the player reporting on itself, cross-checked against capture during the loop-point measurements. |

### Sealed-enclosure thermal test

Conditions: Raspberry Pi 4 in a sealed passive case, no fan, an unheated room in August; a 2560×1440 video at 60 fps into a sink forced to 3840×2160 at 30 Hz; sampled every 30 s for 2 h 04 min.

Result: 199 loops, zero dropped and zero late frames at every heartbeat, throttle bits never set across all 249 samples, peak 78.4 °C, twenty-minute means plateauing at 77.2 °C. The Pi 4 soft-throttles at 80 °C, so a sealed case leaves 1.6 °C. A warmer room or a dust-blocked case removes that margin, and the enclosure needs venting.

Its build predates the change that made the drop counters accumulate across a recovery (see [failure-handling.md](failure-handling.md)), and the output mode did not match the video's frame rate, so the zero-drops reading is the player's own counter under a mode mismatch.

### Pre-flight rate check

Before a long-running test, a script compares mpv's playback position with the wall clock. A 2560×1440 video at 60 fps, 45 Mbps, reads a steady ratio of 1.000, an overall ratio of 0.998 and zero on all three counters — dropped, decoder-dropped and late frames — against 0.954 for the cloud-textured 4K30 test video. Both fail the script's sub-check that playback settle within 20 s (42.1 s and 44.1 s), so that sub-check does not discriminate here.

That file is HEVC Main profile at Level 5, High tier: High tier allows a bitstream buffer of roughly 100 Mbps at that level where Main tier allows 25, so the file's 45 Mbps [VBV](../glossary.md#vbv) cap sits well inside the limit.

### Shorter runs

A 75 s run of a 4K test video against a real DRM display covered about seven health-check ticks with no stall logged and no recovery started. Decode startup runs about 1–3 s behind the display check dex-wait-hdmi(1) performs.

## Recovery and watchdog

### Forced recovery on hardware

Conditions: Raspberry Pi 4, the hardware path (`hwdec=drm`, `drmprime-overlay`, KMS plane), a 4K test file verified 0–89 with no decode failures before the run, recovery forced with `--test-rig-force-recovery-after-secs` under `--test-rig-no-sidecar`. Two runs, bounded at 100 s and 660 s.

The recovery fired once about 15 s after the `loadfile` request in both runs. dexd absorbed the end-of-file event it causes, re-initialised demuxer and decoder without errors, and logged no fatal line. No second recovery fired on its own across 84 s in the first run and about ten minutes in the second, where the health check ticks every ten seconds.

Processor use stayed at 25–27% with process time climbing between samples — what continuous realtime 4K decode costs on this Pi, against near 0% and flat time for a stopped event loop. The chip reported no throttling at 43.8–45 °C. The second run's heartbeat, 9 min 45 s after the recovery, carried the fields `loops=198 uptime=600s temp=45.2C frame-drops=0 vo-delayed=0 pos=584.0s pos-age=0s`.

Not established: that the picture returned to the display.

### Recovery check in CI

The recovery test runs against a real mpv under software decode with no display. As shipped it passes, running its full 30 s deadline. Disabling the recovery counter's increment fails the recovery step in 3.11 s, logging an in-place recovery attempt immediately followed by the fatal playback-ended line. The lint and unit-test steps still pass under the same change: that code path is reachable only through a live mpv event loop.

The check does not cover the hardware decode path, the overlay interop or the plane swap: a container with no GPU skips all three (see [ci.md](ci.md)).

### Watchdog on the device

Conditions: transient systemd units on a Raspberry Pi 4, running as the same unprivileged user as the packaged unit.

| Setup | Result |
|---|---|
| `WatchdogSec=15` with the hang probe `--test-rig-hang-after-secs` | timeout at the limit, process killed with the abort signal, restarted by the unit's restart policy; the identical sequence again 18 s later |
| `WatchdogSec=8`, `ExecStartPre` sleeping 15 s | the timeout fired 8 s after the unit reached started, not 8 s after activation — `ExecStartPre` consumes none of the watchdog budget |
| `WatchdogSec=15`, healthy run with software HEVC decode, sampled every 8 s | the watchdog timestamp advanced every sample at the ping cadence, restarts 0, pings dropped 0 — a healthy run is not killed |

The shipped unit sets `WatchdogSec=180` and dexd pings on its ten-second health-check tick, so eighteen pings fit each window and about seventeen consecutive drops are needed before systemd kills the process (derived). [service-unit.md](service-unit.md) and [failure-handling.md](failure-handling.md) describe the wiring.

Nothing recovered during the twenty-five-hour run, so only tests exercise the counter that tracks overlapping recoveries.

### Packaging checks on hardware

The package installs, enables, starts, stops, removes and purges, with systemd resolving the unit from `/usr/lib/systemd/system/dexd.service`. The arm64 binary embeds no libyaml. `ldd` lists none and `strings` finds no libyaml C symbols — every YAML symbol is Rust-mangled — and no crate in the YAML parser's dependency subtree has a build script, a `links` key, or is a `-sys` crate. See [packaging.md](packaging.md).

The test suite runs on a development workstation and on a Pi before a change lands; counts move with every change, so none is quoted here — [development.md](development.md) says how to run them.

## Capture-instrument limits

### 4K delivery rate

4K delivery is about 27 fps, not 30. Measured over 600 frames in 22.43 s, scaling linearly from 300 frames in 11.04 s, so it is not startup skew. Inter-frame intervals cluster at 0.0358–0.0373 s with no doubled intervals, which is pacing; dropping would show 0.0333 s with occasional 0.0667 s. A different pixel format changes the timing not at all.

The device transmits 4:2:2 over USB whatever is requested, two-thirds the data of 4:4:4 (assumed). 4K30 then needs about 497 MB/s against USB 3.0 Gen 1's practical ceiling of about 450 MB/s, and 450/497 = 0.905 against a measured 27/30 = 0.90.

The [capture deficit](../glossary.md#capture-deficit) is about 10% at 4K and belongs to the instrument, not the player; index analysis tolerates gaps by construction, so the deficit is a design input rather than a fault to chase.

Two further readings show the same pacing. Loop points 3.000 s apart can be detected no later than one capture interval (0.038 s) after they occur, so no measured period should exceed 3.038 s; the measured minimum is 3.0510 s. The step histogram reads +1 ×1377, +2 ×171 (11.0%, matching 30/27) with 72 repeated indices.

### 1080p60 delivery

At 1080p60 the device captures everything: 1800 frames in 30.00 s, none undecodable, at 248 MB/s, which fits USB 3.0 where 4K30 does not. That is the configuration behind every frame-duration histogram above.

### 4K60 capture

Frame rates above 30 Hz at 4K cannot be measured through the device. It records 4K at 30 fps, so at 4K60 it captures every other frame: indices step by 2 throughout, every transition reads anomalous and loop detection breaks.

Its EDID is HDMI 1.4 and caps at 2160p30, a TMDS character rate of 297 MHz; forcing 3840×2160@60 with `hdmi_enable_4kp60=1` leaves it at 297 MHz, because the driver will not synthesise a mode the sink does not advertise. 4K60 needs 594 MHz and an HDMI 2.0 sink, so 4K30 is the measured case and 4K60 is captured at 1080p60 as a weaker check.

### Link speed

A port can renegotiate from SuperSpeed down to USB 2.0 without notice — one did so twice within minutes. An undetected drop mid-run zeroes every captured frame and reads as total frame loss, so a run is valid only with the link at SuperSpeed, checked before and after.

Enumerated at USB 2.0 — behind a hub that itself came up as a USB 2.0 device — the capture device stops advertising 4K input modes: it offers modes according to the bandwidth available.

### Mode behaviour by sink

The capture device lists 3840×2160@30 as its preferred detailed timing, and the vc4 driver builds no 3840×2160 mode from it unforced; forced, the identical timing works. A 2560×1440 monitor must not be given a forced display mode, because transmitting a mode the panel cannot show reads as a player fault. Both cases, and the integer-only refresh grammar that follows from mpv matching modes on [vrefresh](../glossary.md#vrefresh), belong to [exhibit-config.md](exhibit-config.md).

A third sink, observed once on a deployed player and not on the instrument: a Dell U2719DC offers exactly one 2560×1440 timing, at 59.95 Hz. `display_mode: 2560x1440@60` matches no mode there — mpv reports `Could not find mode matching 2560x1440@60` — and the player restarts on it, with `kms_force` already `none`, so this is mpv's own mode matching and not a forced-mode failure. `auto` resolves the connector's preferred timing and plays. A connector whose only timing is fractional therefore has no working integer `display_mode`, and `auto` is the sole value that reaches a picture.

On the same card the 3840×2160 video did not present at all: the clock never advanced and recovery escalated to a restart, while the native 2560×1440 video played. The plane path hands each decoded frame to a KMS plane untouched (see [architecture.md](architecture.md)), which offers no step at which a 3840×2160 frame could be reduced to fit a 2560×1440 mode. This does not contradict the 4K-file-into-a-2560×1440-monitor row under [Bitrate and sink](#bitrate-and-sink): that reading was taken on the detiling path, which converts each frame and can resize it, and which this design replaced for the frame rate it costs. Observed once, on a deployed card.

## Not measured

| Item | Status |
|---|---|
| 500 or more loop points at 4K30 | not tested |
| Colour and geometry (grey ramp, colour wheels, resolution wedges, border) | not automated; checked by eye against the source |
| A long run of the 45 Mbps 1440p60 video | not tested |
| An outside witness that the picture returns after a recovery | not tested |
| Two overlapping recoveries on a device | not tested |
| 4K60 on a Pi 5 | not tested |
| pivid as a player | not tested |
| The Pi 4's HEVC bitrate ceiling | not tested; encodes at 25.3, 39.7 and 45 Mbps all play at realtime, and the roughly 80 Mbps encode-target figure comes from outside this record |
