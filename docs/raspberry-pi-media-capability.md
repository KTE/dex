# Raspberry Pi media capability — which player plays what, up to which quality

**Purpose:** decide, at a glance, whether a given Pi can run a given dex artwork — and whether an
existing installation can be upgraded or must be replaced.

**Rule of the document:** every factual claim carries a source marker `[n]`, resolved in
[Sources](#sources). A claim with no marker is a bug. Claims sourced to `[1]` are **our own
measurements on our own hardware**, not published specification — they are marked `(ours)`
everywhere they appear. Vendor spec, community measurement and marketing claims are labelled as
such and never merged.

---

## 1. TL;DR decision table

| Artwork | Pi 0/1/2 | Pi 3 / Zero 2 W | Pi 4 / 400 / CM4 | Pi 5 / 500 / CM5 |
|---|---|---|---|---|
| 1080p30 H.264 | HW ✅ [9][2] | HW ✅ [10][11] | HW ✅ [7][12] | **SW only** [8][13][38][42] — fine in practice [8] |
| 1080p60 H.264 | ❌ (spec 1080p30) [9] | ❌ (spec 1080p30) [10] | HW ✅ [7][12][15] | SW, ~50–60 % CPU [8] |
| 4K H.264 | ❌ [17][31] | ❌ [17][31][44] | ❌ — HW block clamps at 1920 px [17][31] | SW only; vendor claims viable, unmeasured [42] |
| 1080p HEVC | SW, marginal/thermal-limited [47][29] | SW, marginal (~11–15 Mbit/s ceiling reported) [47][29] | HW ✅ [7][12] | HW ✅ [8][13] |
| 4K30 HEVC 8-bit | ❌ no HEVC HW at all [17][18][29] | ❌ [17][18][29][47] | **HW ✅ realtime — but only on the zero-copy path** (ours) [1] | HW ✅ (vendor 4Kp60) [8][13] |
| 4K60 HEVC | ❌ | ❌ | ⚠️ vendor says 4Kp60 [7][12]; **we measured 0.753× realtime** (ours) [1]; field reports 45–55 fps [46] | vendor 4Kp60 [8][13]; not measured by us |
| VP8 / VP9 / AV1 | ❌ HW, SW infeasible | ❌ HW | ❌ HW (absent from both drivers) [17][21] | SW only; VP9 1080p30 easy, 4K30 "generally fine", 4K60 drops [38] |

**Ten-second rule:** *Pre-Pi-4 = 1080p H.264 only. Pi 4 = the 4K HEVC machine, if and only if the
display path is zero-copy. Pi 5 = HEVC in hardware, everything else on the CPU.*

---

## 2. Generations by SoC (capability is set by the chip, not the model name)

The commonly-cited list "BCM2835 / 2836 / 2837(B0) / 2711 / 2712" is **incomplete**: the Zero 2 W
runs an RP3A0 system-in-package containing a **BCM2710A1** die, which official documentation
describes as the same silicon that is packaged inside BCM2837 [6]. Treat it as a BCM2837-class part,
but do not expect the string "BCM2837" on that board.

| Gen | SoC | Video block | Models |
|---|---|---|---|
| G1 | **BCM2835** [2] | VideoCore IV | Pi 1 A, 1 A+, 1 B, 1 B+; Zero; Zero W; CM1 [2] |
| G2 | **BCM2836** [3] | VideoCore IV — architecture "identical to BCM2835" apart from the CPU cluster [3] | Pi 2 B (early revisions) [3] |
| G3 | **BCM2837** [4] | VideoCore IV @ 400 MHz [4] | Pi 3 B (early); some Pi 2 B; CM3 [4] |
| G3 | **BCM2837B0** [5] | Same silicon as BCM2837; only clock + heat-spreader differ [5] | Pi 2 B (later); Pi 3 B (later); **Pi 3 A+**; **Pi 3 B+**; CM3+ [5] |
| G3 | **BCM2710A1** (in RP3A0 SiP) [6] | Same die as the one inside BCM2837 [6] | **Zero 2 W** only [6] |
| G4 | **BCM2711** [7] | VideoCore VI + **dedicated HEVC block** (`codec@7eb10000`, `brcm,bcm2711-hevc-dec`) [19] | Pi 4 B; **Pi 400**; CM4 [7] |
| G5 | **BCM2712** [8] | VideoCore VII + same-family HEVC block; **no wired H.264 block** [20] | Pi 5; **Pi 500**; Pi 500+; CM5 [8] |

Note on G1–G3: the vendor's own processor pages state the multimedia architecture is unchanged
across BCM2835 → 2836 → 2837 → 2837B0 [2][3][4][5]. Therefore one capability row covers all of them,
even though **per-model product briefs exist only for the endpoints** (Pi 1 B+ [9], Pi 3 B+ [10],
Zero 2 W [11]) — see [Gaps](#8-gaps-what-we-could-not-source).

---

## 3. Hardware decode capability per generation

Two decode engines exist across the whole family, and they are wholly separate pieces of hardware:

- **The legacy VideoCore codec block**, exposed by the `bcm2835-codec` V4L2 M2M (stateful) driver
  over MMAL/VCHIQ firmware [17]. Present on BCM2835 → BCM2711. Absent on BCM2712 [17][20].
- **The Raspberry-Pi-designed HEVC block**, exposed by a V4L2 **stateless** (request API) driver
  (`rpivid`, later `rpi-hevc-dec` / `hevc_d`). First appears on BCM2711; carried forward to
  BCM2712 [19][20][23][24][26].

| Codec | G1–G3 (2835/2836/2837/2837B0/2710A1) | G4 (BCM2711) | G5 (BCM2712) |
|---|---|---|---|
| **H.264** | HW decode, **1080p30** per product briefs [9][10][11]; 1080p60 "generally achievable" but out of spec [31] | HW decode **1080p60**, encode 1080p30 [7][12][15] | **No hardware block at all** [38][42][20]; software decode only [8] |
| **HEVC / H.265** | **None** — not in the driver's format table [17], no DT node [18] | HW decode, vendor "4Kp60" [7][12][15]; driver ceiling 4096×4096 [21][23] | HW decode, vendor "4Kp60" [8][13][14] |
| MPEG-2, VC-1 | Declared by the driver [17] (historically licence-key gated — **not sourced here**, see [Gaps](#8-gaps-what-we-could-not-source)) | Same driver, same list [17] | Software [8] |
| MPEG-4 part 2, H.263 | HW [17][10] | HW [17] | Software [8] |
| MJPEG / JPEG | HW [17] | HW [17] | Software [8] |
| VP8 / VP9 / AV1 | Absent from every RPi decode driver [17][21] | Absent [17][21] | Software (NEON); VP9 1080p30 easy, 4K30 "generally fine", 4K60 drops frames [38] |

### Resolution, profile and bit-depth ceilings

| Limit | Value | Kind | Source |
|---|---|---|---|
| Legacy codec block max dimension | **1920 × 1920** — `MAX_W_CODEC`/`MAX_H_CODEC` hard-coded in the driver | kernel source | [17] |
| H.264 profile accepted by that driver | up to **High** profile | kernel source | [17] |
| H.264 level enumerated by that driver | up to **5.1** — but superseded by the 1920 px clamp above | kernel source | [17] |
| H.264 level the hardware is actually *specified* for | **4.1** (1080p30); 4.2 (1080p60) "generally achievable", not spec | vendor staff, forum | [31] |
| HEVC decoder max frame | **4096 × 4096** — `HEVC_D_MAX_WIDTH/HEIGHT` | kernel source | [21][23] |
| HEVC bit depth | 8-bit **and** 10-bit column-tiled formats exist in the driver (`NV12MT_COL128`, `NV12MT_10_COL128`) — but in the `rpi-6.12.y` source read, the MT variants are **commented out of the offered format list** | kernel source | [21][22] |
| HEVC luma/chroma bit depth | must match — hardware "cannot support having a different bit depth for luma and chroma" | kernel review coverage | [23][25] |
| HEVC bit depth per Kodi/LibreELEC | 8/10/12-bit HDR on Pi 4/400 and Pi 5/500 | downstream project doc | [29] |

**Conflict, stated not resolved:** the vendor spec line "H.265 (4Kp60 decode)" [7][12] is silent on
profile and bit depth; LibreELEC's table claims 8/10/12-bit HDR [29]; the mainline driver source in
the branch we read enumerates only `NV12_COL128` as active [21]. These do not agree, and no vendor
document was found that states a HEVC profile/bit-depth ceiling [see Gaps].

**Two different "4K" claims — keep them apart.** The Pi 4 brief puts *"2 × micro HDMI (up to 4Kp60
supported)"* under **video output** and *"H.265 (4Kp60 decode)"* under **multimedia**, in separate
rows [12]. Display output ≠ decode. Pre-Pi-4 boards fail on **both** counts [44].

### The Mpix/s framing (more predictive than "is it 4K")

Decode throughput on BCM2711 tracks **megapixels per second**, not resolution label (ours) [1]:

| Mode | Mpix/s | Result on Pi 4 (ours) [1] |
|---|---|---|
| 2560×1440p60 HEVC | 221 | **25.5 h continuous, zero dropped frames** |
| 3840×2160p30 HEVC | 249 | realtime, **only on the zero-copy path** |
| 3840×2160p60 HEVC | 498 | **0.753× realtime — under realtime** |

Use ~250 Mpix/s as the practical Pi 4 HEVC budget for a full player pipeline (ours) [1]. If a
proposed artwork exceeds it, it will not hold frame regardless of how the resolution is spelled.
Independent field reports agree qualitatively: a Pi 4B playing a real 4K60 HEVC HDR file under
Kodi measured "mostly 45 to 55 FPS, sometimes as low as 25" and the thread ended unresolved [46].

**Bitrate is a second, separate ceiling.** One community bitrate sweep on Pi 4 / LibreELEC reports
smooth 4K30 HEVC to ~60 Mbit/s, drops at ~70, choppy real-world UHD rips above ~80, and a **hard
freeze above 120–130 Mbit/s**, with the developer attributing it to HEVC intermediate-memory
exhaustion and SDRAM bandwidth rather than CPU (which stayed under 16 %) [45]. Single-source,
not independently reproduced — treat as an order-of-magnitude warning, not a spec.

---

## 4. Player axis — which software reaches which decode path

The reason a player works on one generation and not another is almost always **which decode API it
was written against**. Three mutually incompatible APIs are in play:

| API | What it is | Lives on |
|---|---|---|
| **MMAL / OpenMAX IL** | Proprietary VideoCore firmware interface, 32-bit only | BCM2835 → BCM2711, on Buster-era OS. Removed in Bookworm [37]; OpenMAX unsupported on 64-bit [28][36] |
| **V4L2 M2M (stateful)** | Standard Linux; a wrapper over MMAL on `/dev/video10` | BCM2835 → BCM2711, H.264 etc. [30] |
| **V4L2 stateless / request API** | Userspace parses the bitstream and submits slice params per frame | **HEVC only**, BCM2711 + BCM2712 [21][23][32] |

| Player | Decode API used | G1–G3 | G4 (Pi 4) | G5 (Pi 5) |
|---|---|---|---|---|
| **omxplayer** | OpenMAX IL [28] | ✅ H.264 1080p, its native habitat | 32-bit only; OSD path (OpenVG) unsupported on Pi 4 [28] | ❌ never |
| | **Deprecated since 2020; removed as of Bullseye. Reasons: OpenMAX deprecated and unsupported on 64-bit kernels, no software fallback, no OpenVG on Pi 4. Vendor's named replacement is VLC.** [28][35][36] ||||
| **hello_video** | OpenMAX IL via `ilclient` [27] | ✅ raw H.264 demo | 32-bit/legacy stack only | ❌ — MMAL/OpenMAX removed in Bookworm [37] |
| **VLC** | V4L2 (M2M and stateless) post-Bullseye [35] | ✅ H.264 via M2M | ✅ HEVC HW decode confirmed on Bullseye/Bookworm [39] | ⚠️ reported problems with H.265; **only** the DRM/KMS video output reaches the V4L2 hwaccel — single uncorroborated report [41] |
| **mpv** | V4L2 M2M (`--hwdec=v4l2m2m`) or V4L2-request + DRM_PRIME [49][34] | ✅ H.264 via `v4l2m2m` | ✅ **the working 4K30 HEVC path (ours)** [1] — see below | ✅ HEVC; frame drops reported when the output path detiles [43] |
| **ffmpeg / libav** | `h264_mmal` (legacy, non-zero-copy: VPU decode then memcpy GPU→ARM) [52]; `h264_v4l2m2m`; V4L2-request for HEVC [33] | ✅ `h264_mmal` (needs `gpu_mem ≥ 32M`) [52] | ✅ HEVC via V4L2 stateless in the OS-supplied ffmpeg [39]; `hevc_v4l2m2m` **does not work** — it implements only the stateful API [32] | ✅ HEVC via `-hwaccel drm` (`/dev/video19`), **not** `v4l2m2m` [48] |
| **Kodi / LibreELEC** | v18 = MMAL/OMX; v19+ = V4L2 [29] | Pi 0/1 HEVC unsupported; Pi 2/3 SW-only, SD-limited in LE 10.x [29] | ✅ HW HEVC to 4K, 8/10/12-bit HDR, LE 10.x+ [29] | ✅ same, requires LE 11.x+ [29] |
| **dex-loop** (this project) | ffmpeg V4L2-request HEVC → DRM_PRIME → KMS overlay plane, via mpv `--gpu-hwdec-interop=drmprime-overlay` (ours) [1] | ❌ | ✅ the only realtime 4K30 config we found (ours) [1] | untested by us (ours) [1] |

### Why the interop matters more than the decoder

On BCM2711 the HEVC decoder emits **SAND / column-tiled NV12** (`V4L2_PIX_FMT_NV12_COL128`
family) [21], which the display controller can scan out natively. Any path that does not accept
that layout must **detile** it first, and detiling is where the frames go. Measured by us on a
Pi 4, trixie, 4K30 HEVC (ours) [1]:

| Display path | Result (ours) [1] |
|---|---|
| `drmprime-overlay` (zero-copy scanout) | **realtime, 30 fps** |
| DRM plane path (non-overlay) | 29.1 fps |
| GL import | ~5 fps |
| CPU copy | 14.3 fps |

Independent corroboration of the same failure mode elsewhere: on Pi 5 + Trixie, mpv logging
`VO: [gpu] 1920x1080 yuv420p` — i.e. converting away from the tiled buffer — dropped 500–760 frames
over ~25 s clips [43]; and on Pi 4 + Wayland, `--hwdec=v4l2m2m` produced a blue screen because Mesa
could not import the format, while `v4l2m2m-copy` worked at ~80 % CPU with drops [50].

Structural constraint: the zero-copy KMS plane path requires mpv's native DRM output
(`--gpu-context=drm`) and **cannot run under X11**, because DRM allows a single master [34].
`hello_drmprime` is the minimal reference for that pipeline [53].

### Setup traps that look like "not supported"

- On Raspberry Pi OS the stateless HEVC decoder needs `dtoverlay=rpivid-v4l2` in `config.txt` —
  **not enabled by default**; LibreELEC enables it by default [30].
- Without the overlay, ffmpeg **hard-fails** (`No device available for decoder: device type drm
  needed for codec hevc`) rather than falling back silently — in that specific case [51].
- ffmpeg needs `--enable-v4l2-request --enable-libdrm --enable-vout-drm` at build time for the
  arm64 V4L2-request path [51].
- Raspberry Pi ships a downstream ffmpeg fork with V4L2-request patches; upstream ffmpeg lacked
  full stateless support as of the driver's kernel submission [33][23]. Whether upstream has merged
  it by 2026 is **unverified** [see Gaps].

---

## 5. Max's two questions, answered

### A. Could anything earlier than the Pi 4 play 4K in hardware — at all?

**No. Confidence: high.** Three independent lines of evidence, none of which contradict:

1. **Vendor spec, per generation.** Every pre-BCM2711 product brief tops out at 1080p30 H.264 /
   MPEG-4 decode and never mentions 4K or HEVC: Pi 1 B+ [9], Pi 3 B+ [10], Zero 2 W [11]. The
   processor pages state the multimedia architecture is common across BCM2835→2837B0 [2][3][4][5].
2. **Kernel source, structural.** The only hardware decode driver those chips have,
   `bcm2835-codec`, hard-codes a **1920 × 1920 ceiling** for the codec role [17] — so 4K is
   impossible *for any codec*, H.264 included, on everything up to and including the Pi 4's legacy
   block. And HEVC is not merely limited but **absent**: no HEVC entry in that driver's format
   table [17], and no HEVC device-tree node in `bcm2835.dtsi` / `bcm2836.dtsi` / `bcm2837.dtsi` /
   `bcm283x.dtsi` [18].
3. **Vendor engineer, on record.** On a "4K video on Pi 3 B+" thread: *"The HW only supports up to
   1080p60 (a tiny bit more with overclocking IIRC)"*, and separately that even a successful
   software decode is moot because *"the HDMI output is realistically limited to 1080P60"* [44].

So the failure is **doubled**: decode ceiling *and* display-output ceiling [44]. Software 4K decode
on a Cortex-A53 is not a workaround worth testing — a Pi 3 already cannot sustain **1080p** HEVC in
software, going thermal-limited and buffering at ~11 Mbit/s, with a stronger report of ~15 Mbit/s
max at 100 % CPU [47]. LibreELEC's own table caps Pi 2/3 at software SD in LE 10.x, and states the
optimised HEVC decode code from LE 9.2.x was dropped and "won't come back" [29].

**Not worth testing.** Nothing earlier than a Pi 4 is a candidate for 4K dex artwork.

*Caveat on completeness:* the community threads found cover the **Pi 3** directly [44][47]. No
report was located of anyone attempting 4K on a Pi 0/1/2 specifically — their exclusion rests on
the shared-silicon vendor statements [2][3][4] and the driver ceiling [17], which is strong, but is
inference rather than a direct test [see Gaps].

### B. Did the Pi 5 lose H.264 hardware decode relative to the Pi 4?

**Yes. The BCM2712 has no usable H.264 hardware decode (or encode). Confidence: very high —
this is confirmed at primary-source level in kernel source *and* by named vendor engineers.**

Evidence, strongest first:

| # | Evidence | Kind |
|---|---|---|
| 1 | `bcm2712.dtsi` IOMMU node carries the comment `/* IOMMU2 for PISP-BE, HEVC; and (unused) H264 accelerators */` — the H.264 accelerator is **on the die but unwired**; no H.264 DT node, driver binding or register range is exposed anywhere in the tree [20] | kernel source |
| 2 | RPi engineer *jamesh*: *"The 2712 does NOT have a H264 HW block for encoding or decoding"* [38]; and Gordon Hollingworth in the Pi 5 launch post comments: *"H264 hardware decoding has been removed"* [16] | vendor staff, informal |
| 3 | Official BCM2712 docs: headline feature is "4Kp60 HEVC hardware decode", followed by "**Other CODECs run in software**" and CPU-load figures — *H.264 1080p24 ≈ 10–20 % CPU, 1080p60 ≈ 50–60 % CPU* [8] | vendor doc |
| 4 | Structural silence: the Pi 5 product brief has **no Multimedia row at all** and the string "H.264" never appears; the Pi 500 brief lists only "H.265 (4Kp60 decode)" — a sharp break from Pi 3 B+, Zero 2 W and Pi 4 briefs, which all carry an explicit H.264 line [13][14] vs [10][11][12] | vendor doc, by omission |
| 5 | The `bcm2835-codec` driver — the only thing that ever provided H.264 decode — binds via VCHIQ/firmware platform device, not device tree [17]; BCM2712 does not run that firmware stack at all [17] | kernel source |

**Nuance that matters for dex's existing 1080p H.264 artworks.** No official vendor document frames
this as a regression *relative to the Pi 4* — the loss is stated as a fact about BCM2712 in
isolation [8][13] [see Gaps]. The vendor's position is that it does not matter:

- *dom* (RPi engineer): *"The Pi5 can decode H.264 faster in software than the Pi4 can decode in
  hardware"*, calling the removed block a design "over 15 years" old [42].
- *jamesh*: the quad Cortex-A76s are *"capable of higher resolution and higher quality decodes than
  the HW block on the previous models"*; the Pi 4's block was 1080p-limited, whereas Pi 5 can do
  4K H.264 in software [42].

**Label this correctly: that is a vendor claim, not a measurement.** No fps or CPU-headroom number
is attached to it in the thread [42], and no independent player-pipeline benchmark comparing Pi 4
H.264-hardware vs Pi 5 H.264-software was found in this research [see Gaps]. The known-good number
is the vendor's own CPU-load figure — **1080p60 H.264 costs 50–60 % of the Pi 5's CPU** [8] — which
is fine for a dedicated single-artwork player and *not* fine if anything else shares the box.

**Practical answer for dex:** existing 1080p H.264 artworks are very likely fine on a Pi 5, but they
run on the CPU there, with no zero-copy tiled path and no thermal headroom guarantee. Two actions
follow: (a) **measure before deploying** an H.264 artwork on Pi 5, exactly as we measured HEVC on
Pi 4 (ours) [1]; (b) for anything new, **author in HEVC** — it is the only codec with a hardware
path on both BCM2711 and BCM2712 [19][20][23], and the same driver covers both (ours) [1][23].

---

## 6. dex project measurements (ours — primary evidence for BCM2711)

Measured on a Pi 4 / BCM2711, Debian trixie, August 2026. Log:
`experiments/2026-08-12-4k-hevc-perfect-loop/LOG-4k-hevc-perfect-loop.md`. These are **our numbers,
not published specification** [1].

| Finding | Number | Notes |
|---|---|---|
| 4K30 HEVC, `drmprime-overlay` zero-copy | **realtime** | decoder emits SAND-tiled NV12; display scans it out natively |
| 4K30 HEVC, DRM plane (non-overlay) | 29.1 fps | detiles |
| 4K30 HEVC, GL import | ~5 fps | detiles |
| 4K30 HEVC, CPU copy | 14.3 fps | detiles |
| 4K60 HEVC | **0.753× realtime** | under realtime — Pi 4 cannot do 4K60 in a real pipeline |
| 2560×1440p60 HEVC | 25.5 h, **zero dropped frames** | 221 Mpix/s |
| Driver | `rpi-hevc-dec`, V4L2 **stateless** | covers both BCM2711 and BCM2712 |

**Where ours and the vendor disagree:** vendor spec says BCM2711 does "H.265 (4Kp60 decode)"
[7][12][15]; we measured 4K60 at 0.753× realtime (ours) [1], and an independent Kodi user measured
45–55 fps on real 4K60 HDR content [46]. The vendor number plausibly describes the decode block in
isolation; it is not achievable through a complete decode-and-present pipeline on the configurations
either of us tested. **Do not plan a 4K60 artwork on a Pi 4.**

Corroboration of our zero-copy finding in the driver itself: the capture-side native format is the
column/SAND-tiled NV12 family [21], which is exactly why every non-tiled interop path pays a detile
cost.

---

## 7. How to check on your own device (do not assume — verify)

A silent software fallback looks exactly like success. mpv's `--hwdec-software-fallback` defaults to
reverting to software decode after 3 consecutive hardware failures [49] — and whether that is
logged by default is **not documented** in the page we read [49] [see Gaps]. So check, every time.

```bash
# 1. Which decode devices exist at all?
ls -l /dev/video*                 # video10/11/12 = legacy M2M; video19 = stateless HEVC (Pi 5) [48]
v4l2-ctl --list-devices

# 2. What does the HEVC decoder actually advertise?
v4l2-ctl -d /dev/video19 --list-formats-out   # expect S265 / HEVC_SLICE  -> stateless [40][21]
v4l2-ctl -d /dev/video19 --list-formats       # expect NC12 / NV12_COL128 -> tiled capture [40][21]

# 3. Is the stateless overlay even loaded? (Raspberry Pi OS: NOT default) [30]
grep -n rpivid /boot/firmware/config.txt      # want: dtoverlay=rpivid-v4l2
dmesg | grep -iE 'rpivid|hevc'

# 4. Does ffmpeg have the path compiled in?
ffmpeg -hide_banner -buildconf | grep -E 'v4l2-request|libdrm|vout-drm'   # [51]
ffmpeg -hide_banner -decoders | grep -iE 'hevc|h264'

# 5. THE REAL TEST — is hardware decode engaged, and is the interop zero-copy?
mpv --hwdec=auto --gpu-context=drm --gpu-hwdec-interop=drmprime-overlay \
    --msg-level=all=v FILE.mkv 2>&1 | grep -iE 'hwdec|drmprime|Using hardware|fallback|Falling back'
#   PASS: an explicit "Using hardware decoding" line naming the drmprime path.
#   FAIL: any 'falling back' line, OR a VO line showing a planar format such as
#         "VO: [gpu] 3840x2160 yuv420p" -> the buffer got detiled, frames are being lost [43]

# 6. Prove it with numbers, not vibes: play a fixed-length clip and count.
mpv --untimed=no --msg-level=all=v FILE.mkv 2>&1 | tail -5   # check dropped-frame count
#   Or watch CPU: hardware HEVC decode should be single-digit-to-low % CPU.
#   1080p60 H.264 on a Pi 5 at ~50-60% CPU is *expected* (software) [8], not a bug.
```

**Interpretation rule:** hardware decode with a detiling interop is *worse than useless* for
diagnosis — it reports "hardware decoding" and still drops most frames (ours) [1][43]. The frame
counter is the ground truth, not the hwdec log line.

---

## 8. Gaps — what we could not source

Stated as gaps rather than guessed.

1. **CM4, CM5, Pi 400 product briefs** were not fetched. Their capability rows here are inferred
   from shared-SoC statements [7][8], not from their own documents.
2. **Per-model briefs for Pi 2 B (either revision), Zero, Zero W, CM1, CM3, CM3+, original Pi 3 B,
   Pi 3 A+** do not exist in this evidence set. Raspberry Pi Ltd appears to have retired several of
   the oldest briefs (only mechanical drawings remain live for the Pi 1 B+, which is why [9] is an
   Adafruit mirror, not a live raspberrypi.com URL).
3. **HEVC profile / bit-depth ceiling on BCM2711:** no vendor document states 8-bit vs Main10.
   Sources conflict (see §3). The BCM2711 peripherals datasheet (RP-008248-DS) was located but not
   read; it is the likely place this lives.
4. **A public BCM2712 datasheet** was not located and may not exist — the decoder block is
   Raspberry Pi-designed rather than licensed VideoCore. Unconfirmed either way.
5. **No vendor sentence frames the Pi 5 H.264 loss as a regression vs the Pi 4.** The conclusion in
   §5B is composed from several documents plus engineer statements; it is very strong but is not a
   single-source vendor claim.
6. **No independent benchmark** of Pi 4 H.264-hardware vs Pi 5 H.264-software on the same clip.
   The vendor comparative claim [42] is unevidenced with numbers.
7. **Contradiction left open:** a Pi 5 V4L2 thread shows a `v4l2-ctl` capability listing capped at
   1920×1088 while the same poster reports working 4K60 HEVC in their project [40]. Probably one
   enumerated mode rather than the driver ceiling — the driver constant is 4096×4096 [21][23] — but
   the thread as fetched does not resolve it.
8. **MPEG-2 / VC-1 licence keys:** the historical paid-unlock requirement is widely known but no
   kernel-source or vendor citation was found in this pass, so it is not asserted here.
9. **Upstream ffmpeg V4L2-request merge status as of 2026** is unverified; as of the driver's kernel
   submission it had not landed [23][33].
10. **mpv software-fallback logging:** the docs do not state whether the fallback is announced
    [49]. Assume it may be silent — hence §7.
11. **ffplay** zero-copy KMS support on Pi: no primary source found; unknown, not asserted.
12. **VLC on Pi 5 + HEVC** rests on a single uncorroborated August 2025 forum report [41].
13. **No Pi 5 measurements of our own** — everything in §5B about Pi 5 behaviour is vendor or
    community sourced, not measured by us.

---

## Sources

Access dates as recorded by the researchers. "ours" marks this project's own primary measurements.

**Project measurements (ours — primary, unpublished)**

1. dex project measurements, Pi 4 / BCM2711, Debian trixie, August 2026 —
   `experiments/2026-08-12-4k-hevc-perfect-loop/LOG-4k-hevc-perfect-loop.md` (this repo,
   branch `experiment/4k-hevc-perfect-loop`). Measured 2026-08.

**Vendor documentation and specifications**

2. Raspberry Pi Documentation — Processors: BCM2835 (adoc source).
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2835.adoc — accessed 2026-08-17
3. Raspberry Pi Documentation — Processors: BCM2836.
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2836.adoc — accessed 2026-08-17
4. Raspberry Pi Documentation — Processors: BCM2837.
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2837.adoc — accessed 2026-08-17
5. Raspberry Pi Documentation — Processors: BCM2837B0.
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2837b0.adoc — accessed 2026-08-17
6. Raspberry Pi Documentation — Processors: RP3A0 (BCM2710A1 die, Zero 2 W).
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/rp3a0.adoc — accessed 2026-08-17
7. Raspberry Pi Documentation — Processors: BCM2711.
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2711.adoc — accessed 2026-08-17
8. Raspberry Pi Documentation — Processors: BCM2712.
   https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2712.adoc — accessed 2026-08-17
   *(compiled equivalent, identical wording: https://www.raspberrypi.com/documentation/computers/processors.html — accessed 2026-08-17)*
9. Raspberry Pi Model B+ product brief (Raspberry Pi Foundation document, **third-party Adafruit
   mirror** — the original is no longer served from raspberrypi.com).
   https://cdn-shop.adafruit.com/datasheets/pi-specs.pdf — accessed 2026-08-17
10. Raspberry Pi 3 Model B+ product brief, RP-008338-DS-2, published October 2025, Raspberry Pi Ltd.
    https://pip-assets.raspberrypi.com/categories/532-raspberry-pi-3-model-b/documents/RP-008338-DS-2-raspberry-pi-3-b-plus-product-brief.pdf — accessed 2026-08-17
11. Raspberry Pi Zero 2 W product brief, RP-008359-DS-1, published April 2024, Raspberry Pi Ltd.
    https://pip-assets.raspberrypi.com/categories/584-raspberry-pi-zero-2-w/documents/RP-008359-DS-1-raspberry-pi-zero-2-w-product-brief.pdf — accessed 2026-08-17
12. Raspberry Pi 4 Model B product brief, RP-008344-DS-5, published April 2026, Raspberry Pi Ltd.
    https://pip-assets.raspberrypi.com/categories/545-raspberry-pi-4-model-b/documents/RP-008344-DS-5-raspberry-pi-4-product-brief.pdf — accessed 2026-08-17
13. Raspberry Pi 5 product brief, RP-008348-DS-6, published April 2026, Raspberry Pi Ltd.
    https://pip-assets.raspberrypi.com/categories/892-raspberry-pi-5/documents/RP-008348-DS-6-raspberry-pi-5-product-brief.pdf — accessed 2026-08-17
14. Raspberry Pi 500 product brief, RP-008349-DS-4, published April 2026, Raspberry Pi Ltd.
    https://pip-assets.raspberrypi.com/categories/1115-raspberry-pi-500/documents/RP-008349-DS-4-raspberry-pi-500-product-brief.pdf — accessed 2026-08-17
15. Raspberry Pi 4 Model B specifications page (marketing/spec sheet).
    https://www.raspberrypi.com/products/raspberry-pi-4-model-b/specifications/ — accessed 2026-08-17
16. "Introducing: Raspberry Pi 5!" — raspberrypi.com news post, incl. engineer comments
    (Gordon Hollingworth). https://www.raspberrypi.com/news/introducing-raspberry-pi-5/ — accessed 2026-08-17

**Kernel and driver source**

17. `drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c`, raspberrypi/linux,
    branch `rpi-6.6.y` — format table, `MAX_W_CODEC`/`MAX_H_CODEC` = 1920, H.264 profile/level
    controls, platform-driver binding.
    https://github.com/raspberrypi/linux/blob/rpi-6.6.y/drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c — accessed 2026-08-17
18. `arch/arm/boot/dts/broadcom/` (bcm2835.dtsi, bcm2836.dtsi, bcm2837.dtsi, bcm283x.dtsi),
    raspberrypi/linux `rpi-6.6.y` — no `hevc` string, no `codec@` decoder node.
    https://github.com/raspberrypi/linux/tree/rpi-6.6.y/arch/arm/boot/dts/broadcom — accessed 2026-08-17
19. `bcm2711.dtsi`, raspberrypi/linux `rpi-6.12.y` — `hevc_dec: codec@7eb10000`,
    compatible `brcm,bcm2711-hevc-dec` / `raspberrypi,hevc-dec`.
    https://github.com/raspberrypi/linux/blob/rpi-6.12.y/arch/arm/boot/dts/broadcom/bcm2711.dtsi — accessed 2026-08-17
20. `bcm2712.dtsi`, raspberrypi/linux `rpi-6.6.y` — HEVC node
    (`raspberrypi,rpivid-vid-decoder`) and the IOMMU comment
    `/* IOMMU2 for PISP-BE, HEVC; and (unused) H264 accelerators */`.
    https://github.com/raspberrypi/linux/blob/rpi-6.6.y/arch/arm64/boot/dts/broadcom/bcm2712.dtsi — accessed 2026-08-17
    *(Node naming is branch-dependent; the `rpi-6.12.y` BCM2712 file was restructured and the node
    was not located there in this pass.)*
21. `drivers/media/platform/raspberrypi/hevc_dec/hevc_d_video.c` (and `hevc_d.c`),
    raspberrypi/linux `rpi-6.12.y` — `HEVC_D_MAX_WIDTH/HEIGHT` = 4096, `V4L2_PIX_FMT_HEVC_SLICE`
    output, `NV12_COL128` / `NV12MT_COL128` / `NV12MT_10_COL128` capture formats.
    https://github.com/raspberrypi/linux/blob/rpi-6.12.y/drivers/media/platform/raspberrypi/hevc_dec/hevc_d_video.c — accessed 2026-08-17
22. `rpi-hevc-dec` upstream patch series (binding + new pixel formats), patchew archive.
    https://patchew.org/linux/20241220-media-rpi-hevc-dec-v1-0-0ebcc04ed42e@raspberrypi.com/20241220-media-rpi-hevc-dec-v1-4-0ebcc04ed42e@raspberrypi.com/ — accessed 2026-08-17
23. "Raspberry Pi HEVC decoder driver" — LWN.net coverage of the upstream submission
    (4096×4096 ceiling, luma/chroma bit-depth constraint, v4l2-compliance 142/147 on 6.16.0,
    GStreamer MR 9247, ffmpeg still downstream-patched).
    https://lwn.net/Articles/1028029/ — accessed 2026-08-17
24. "Raspberry Pi HEVC Decoder Driver Posted For Linux Kernel Review" — Phoronix
    (driver covers BCM2711 **and** BCM2712; exposed as a stateless V4L2 decoder device).
    https://www.phoronix.com/news/Raspberry-Pi-HEVC-H265-Decode — accessed 2026-08-17
25. LWN.net follow-up coverage of the same submission — cover letter wording
    ("found in the BCM2711 and BCM2712 processors"), new `NV12MT_COL128` / `NV12MT_10_COL128`
    formats, v4l2-compliance failures above the driver's 4096×4096 limit.
    https://lwn.net/Articles/1060711/ — accessed 2026-08-17
26. "V4L2 HEVC driver" by 6by9 — raspberrypi/linux PR #3505 (original `rpivid` staging driver for
    BCM2711, merged to `rpi-5.4.y` 2020-03-27).
    https://github.com/raspberrypi/linux/pull/3505/files — accessed 2026-08-17
27. `hello_video` source (OpenMAX IL via ilclient), raspberrypi/userland.
    https://github.com/raspberrypi/userland/blob/master/host_applications/linux/apps/hello_pi/hello_video/video.c — accessed 2026-08-17
28. popcornmix/omxplayer README (deprecation notice and stated reasons).
    https://github.com/popcornmix/omxplayer/blob/master/README.md — accessed 2026-08-17
29. LibreELEC documentation — Raspberry Pi hardware (per-model HEVC support table; Kodi v19
    MMAL/OMX removal).
    https://github.com/LibreELEC/documentation/blob/master/hardware/raspberry-pi.md — accessed 2026-08-17

**Raspberry Pi forums (vendor engineers and community — labelled in-text)**

30. "STICKY: All about accelerated video on the Raspberry Pi" — `dtoverlay=rpivid-v4l2` not default
    on RPi OS; V4L2 M2M stateful wrapper over MMAL on `/dev/video10`.
    https://forums.raspberrypi.com/viewtopic.php?t=317511 — accessed 2026-08-17
31. "H264 video decoding" — H.264 spec'd at level 4.1; 4.2 "generally achievable", never 4K.
    https://forums.raspberrypi.com/viewtopic.php?t=298607 — accessed 2026-08-17
32. "Should v4L2 enable HW decode of hevc/h265?" — HEVC reachable only via the stateless request
    API; `hevc_v4l2m2m` will not work.
    https://forums.raspberrypi.com/viewtopic.php?t=296736 — accessed 2026-08-17
33. "FFmpeg HWA for Transcoding" — RPi ships vendor kernel + downstream ffmpeg fork for V4L2-request.
    https://forums.raspberrypi.com/viewtopic.php?t=331026 — accessed 2026-08-17
34. "RPI 4 h265 decoding display with overlay" — DRM single-master constraint; zero-copy plane path
    requires `--gpu-context=drm`, incompatible with X11.
    https://forums.raspberrypi.com/viewtopic.php?t=332651 — accessed 2026-08-17
35. "Replacing OMXPlayer with VLC" — omxplayer removed as of Bullseye; VLC is the named replacement;
    MMAL/OpenMAX code must be rewritten for V4L2.
    https://forums.raspberrypi.com/viewtopic.php?t=336535 — accessed 2026-08-17
36. "OMXPlayer — why it's no longer there?" — OpenMAX is a 32-bit-only firmware interface.
    https://forums.raspberrypi.com/viewtopic.php?t=346146 — accessed 2026-08-17
37. "A little bit on RPiOS 'Bookworm'" — MMAL and OpenMAX removed entirely in Bookworm;
    `hello_video` and similar no longer work.
    https://forums.raspberrypi.com/viewtopic.php?t=352477 — accessed 2026-08-17
38. "RPi5 Codec confusion" — jamesh (RPi Engineer): *"The 2712 does NOT have a H264 HW block for
    encoding or decoding"*; 6by9 on VP9/AV1/H.264 software decode.
    https://forums.raspberrypi.com/viewtopic.php?t=357870 — accessed 2026-08-17
39. "HEVC Support on RPi 4/5" — VLC, Kodi and OS-supplied ffmpeg all reach HEVC HW decode on
    Pi 4/400 (Oct 2024); Pi 5 status uncertain to the responder at that date.
    https://forums.raspberrypi.com/viewtopic.php?t=377567 — accessed 2026-08-17
40. "[SOLVED] Decoding H265 on Raspberry Pi 5 via V4L2" — `rpivid`, `/dev/video19`, S265/NC12,
    stateless request API; **internally inconsistent** on the 1920×1088 vs 4K60 question.
    https://forums.raspberrypi.com/viewtopic.php?t=381601 — accessed 2026-08-17
41. "RPI5 — VLC — HEVC/H265 Playback" — single community report (Aug 2025) that only VLC's DRM/KMS
    output reaches the V4L2 hwaccel on Pi 5.
    https://forums.raspberrypi.com/viewtopic.php?t=390492 — accessed 2026-08-17
42. "Raspberry Pi 5 and the Lack of Hardware H.264 Decoding – A Huge Step Backward" — dom and
    jamesh (RPi Engineers) on software-vs-hardware H.264. **Vendor claim, no numbers.**
    https://forums.raspberrypi.com/viewtopic.php?t=391283 — accessed 2026-08-17
43. "RPi 5: MPV dropping frames" — Trixie/mpv field measurement; `VO: [gpu] … yuv420p` detile
    signature; 7 drops at 1080p60 output vs 176 at 4K60 vs 500–760 at 3440×1440.
    https://forums.raspberrypi.com/viewtopic.php?t=393942 — accessed 2026-08-17
44. "4K videos on Rasp Pi 3 B+" — RPi engineers: HW supports up to 1080p60; HDMI output also
    limited to 1080p60. **Primary answer to Question A.**
    https://forums.raspberrypi.com/viewtopic.php?t=223801 — accessed 2026-08-17

**Independent measurements and third-party reports**

45. "RPI4 HEVC playback max bitrate" — LibreELEC Forum. Single-user bitrate sweep:
    ~60 Mbit/s smooth, ~70 drops, >80 choppy, 120–130 freeze; developer attributes to HEVC
    intermediate-memory / SDRAM bandwidth. **Not independently reproduced.**
    https://forum.libreelec.tv/thread/22076-rpi4-hevc-playback-max-bitrate/ — accessed 2026-08-17
46. "Raspberry Pi 4B not reaching 4K 60FPS when playing HEVC HDR movie" — LibreELEC Forum.
    Measured 45–55 fps, dipping to 25, ~60 % CPU; unresolved. **Corroborates our 4K60 result.**
    https://forum.libreelec.tv/thread/24403-raspberry-pi-4b-not-reaching-4k-60fps-when-playing-hevc-hdr-movie-9-97-1-fresh-i/ — accessed 2026-08-17
47. "HEVC / 265 files on a Pi 3B, around 11Mbit, it can't keep up" — LibreELEC Forum.
    Pi 3B thermal-limited and buffering at ~11 Mbit/s 1080p HEVC; Pi 3B+ with heatsinks ~15 Mbit/s
    at 100 % CPU; LE 9.2.x HEVC optimisations dropped in LE 10 and "won't come back".
    https://forum.libreelec.tv/thread/17577-hevc-265-files-on-a-pi-3b-around-11mbit-it-can-t-keep-up/ — accessed 2026-08-17
48. "Raspberry Pi 5 H265 HEVC Hardware Decoding Working" — Frigate GitHub Discussion.
    Pi 5 needs `-hwaccel drm` (not `v4l2m2m`) on `/dev/video19`; CPU ~45 % → ~20 % after switching
    an H.265 stream to hardware decode; no H.264 hardware transcode on Pi 5.
    https://github.com/blakeblackshear/frigate/discussions/18431 — accessed 2026-08-17
49. mpv — Hardware Decoding documentation (`--hwdec=auto` whitelist;
    `--hwdec-software-fallback` default 3 frames; logging behaviour undocumented).
    https://mpv-player-mpv.mintlify.app/av/hardware-decoding — accessed 2026-08-17
50. mpv issue #10956 — Pi 4 + Wayland: `v4l2m2m` direct yields blue screen
    ("unsupported DRM image format yuv420p"); `v4l2m2m-copy` works at ~80 % CPU with drops.
    https://github.com/mpv-player/mpv/issues/10956 — accessed 2026-08-17
51. jellyfin-ffmpeg issue #129 — Pi 4 64-bit V4L2-request setup: `dtoverlay rpivid-v4l2` plus
    `--enable-v4l2-request --enable-libdrm --enable-vout-drm`; hard failure without the overlay.
    https://github.com/jellyfin/jellyfin-ffmpeg/issues/129 — accessed 2026-08-17
52. "[FFmpeg-user] Using h264_mmal decoder on Raspberry Pi 4" — `h264_mmal` requires ≥32 MB
    `gpu_mem`; VPU decode then memcpy GPU→ARM, i.e. **not** zero-copy.
    https://www.mail-archive.com/ffmpeg-user@ffmpeg.org/msg23170.html — accessed 2026-08-17
53. jc-kynesim/hello_drmprime — minimal DRM_PRIME → KMS zero-copy reference for the
    V4L2-request HEVC path.
    https://github.com/jc-kynesim/hello_drmprime — accessed 2026-08-17
