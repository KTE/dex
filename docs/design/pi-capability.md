# Raspberry Pi media capability

This page records which Raspberry Pi hardware decodes and displays what, for a developer choosing a board or judging whether a proposed video will play. Vendor documentation, third-party reports and this project's own measurements stay apart; the unresolved items are listed at the end.

## Evidence rules

Every factual claim carries a source marker `[n]` into the list at the end, or a provenance label. A claim established by this project carries *measured* with the board named and is never merged into a vendor or community figure; [the measurement record](measurements.md) states the conditions. An unsourced or contradicted claim becomes an item on the measurement to-do list below, with the project's own test setup as the intended source (decided).

Vendor product briefs exist for only three boards before the Pi 4: the Pi 1 Model B+, the Pi 3 Model B+ and the Zero 2 W [9][10][11]. The Sources section below gives each cited brief its document number and publish date [10][11][12][13][14].

## Chips and boards

Capability follows the chip, not the board name.

| Chip | Boards | Video block |
|---|---|---|
| BCM2835 | Pi 1 A/A+/B/B+, Zero, Zero W, CM1 | VideoCore IV |
| BCM2836 | Pi 2 Model B (early) | VideoCore IV |
| BCM2837 | Pi 3 Model B (early), some Pi 2 Model B, CM3 | VideoCore IV at 400 MHz |
| BCM2837B0 | Pi 2 Model B (late), Pi 3 A+/B+, CM3+ | The BCM2837 silicon; clock and heat spreader differ |
| BCM2710A1, in the RP3A0 package | Zero 2 W | The BCM2837 die, repackaged |
| BCM2711 | Pi 4 Model B, Pi 400, CM4 | VideoCore VI plus a dedicated HEVC block |
| BCM2712 | Pi 5, Pi 500, Pi 500+, CM5 | VideoCore VII plus a same-family HEVC block; the H.264 block is unwired |

Sources: the vendor processor pages [2][3][4][5][6][7][8], the HEVC device-tree node [19] and the absent H.264 block on BCM2712 [20]. The Zero 2 W's RP3A0 is a system-in-package holding the same silicon as BCM2837, which the commonly listed BCM2835/2836/2837(B0)/2711/2712 series omits [6].

Those pages state no video-block change from BCM2835 to BCM2837B0 — only the CPU cluster, clock and packaging differ — so this page treats the multimedia block as unchanged (derived) [2][3][4][5].

## Decode engines and APIs

Two decode engines exist across the family, with no shared driver.

- **The legacy VideoCore codec block.** A V4L2 stateful memory-to-memory driver, `bcm2835-codec`, on the MMAL firmware interface; BCM2835 through BCM2711, absent on BCM2712 [17].
- **The Raspberry Pi HEVC block.** A V4L2 stateless driver, named in turn `rpivid`, `rpi-hevc-dec` and `hevc_d`; BCM2711 and BCM2712 [19][20][23][24][26].

The upstream `rpi-hevc-dec` patch series describes the decoder as "found in the BCM2711 and BCM2712 processors", so one driver covers both generations [22].

| Kernel branch | Chip | Node and compatible strings |
|---|---|---|
| `rpi-5.4.y` | BCM2711 | The original `rpivid` staging driver, merged there [26] |
| `rpi-6.12.y` | BCM2711 | `hevc_dec: codec@7eb10000`; `brcm,bcm2711-hevc-dec` and `raspberrypi,hevc-dec` [19] |
| `rpi-6.6.y` | BCM2712 | `raspberrypi,rpivid-vid-decoder`; `rpi-6.12.y` restructured that file and the equivalent node is not cited here (not tested) [20] |

Three mutually incompatible decode APIs reach these engines.

| API | Scope | Note |
|---|---|---|
| MMAL and OpenMAX IL | The proprietary VideoCore firmware interface, 32-bit only, BCM2835 through BCM2711 | OpenMAX is deprecated and unsupported on 64-bit kernels [28][35] |
| V4L2 stateful (memory-to-memory) | A standard Linux wrapper over MMAL, BCM2835 through BCM2711, H.264 and the older codecs [30] | The cited source gives no device-node number |
| V4L2 stateless (request API) | HEVC only, BCM2711 and BCM2712 [21][23][32] | Userspace parses the bitstream and submits slice parameters per frame |

## Capability matrix

| Video | BCM2835–2837B0, RP3A0 | BCM2711 | BCM2712 |
|---|---|---|---|
| H.264 1080p30 | Hardware [9][10][11] | Hardware [7][12] | Software; workable in practice [8][13][37][41] |
| H.264 1080p60 | Outside the hardware specification (level 4.0, 1080p30); some streams decode [9][10][31] | Hardware [7][12][15] | Software, roughly 50–60 % of the CPU [8] |
| H.264 3840×2160 | Fails; the driver clamps at 1920 px [17] | Fails on the same clamp [17] | Software; a vendor claim with no measurement [41] |
| HEVC 1080p | Software, marginal and thermally limited [46] | Hardware [7][12] | Hardware [8][13] |
| HEVC 3840×2160p30, 8-bit | Fails; no HEVC hardware [17][18] | Hardware; realtime on the zero-copy path alone (measured on a Raspberry Pi 4) [1] | Vendor claims 4Kp60 [8][13] |
| HEVC 3840×2160p60 | Fails [17][18] | 0.753× realtime (measured on a Raspberry Pi 4) [1]; an independent report gives 45–55 fps [45] | Vendor claims 4Kp60 [8][13] |
| MPEG-2, VC-1 | Declared by the legacy driver [17] | Declared by the legacy driver [17] | Software [8] |
| MPEG-4 part 2, H.263, MJPEG, JPEG | Hardware [17][10] | Hardware [17] | Software [8] |
| VP8, VP9, AV1 | No hardware decode; software is infeasible [17] | No hardware decode [17][21] | Software; VP9 at 1080p30 is easy, 4K30 generally fine, 4K60 drops frames [37] |

The historical paid unlock for MPEG-2 and VC-1 decode is widely reported; no kernel source or vendor document states it, so this page does not assert it (not tested).

### Dimension and format limits

The legacy codec block decodes at most 1920 × 1920 px, fixed in the driver as `MAX_W_CODEC` and `MAX_H_CODEC` [17]. That clamp rules out 4K for every codec on that block, H.264 included, through to the Pi 4's legacy path.

The same driver accepts H.264 profiles up to High and enumerates levels up to 5.1 [17], which the clamp supersedes. The hardware specification is level 4.0 and 1080p30, so 1080p60 streams may decode without being covered by it: "It can decode many 1080p60 streams, but you need to handle it efficiently", a Raspberry Pi engineer writes [31].

The HEVC decoder's largest frame is 4096 × 4096 px, set by the kernel constants `HEVC_D_MAX_WIDTH` and `HEVC_D_MAX_HEIGHT` [21][23]. The fifth revision of the `rpi-hevc-dec` patch series adds the tiled formats `NV12MT_COL128` and `NV12MT_10_COL128`, and its cover letter reports failures above that limit in `v4l2-compliance`, the V4L2 conformance test suite [25]. Luma and chroma must share one bit depth [23][25].

The bit-depth ceiling stays unresolved; three sources disagree.

- The vendor line "H.265 (4Kp60 decode)" states no profile and no bit depth [7][12].
- LibreELEC's table claims 8, 10 and 12 bits with high dynamic range [29].
- The `rpi-6.12.y` driver source offers only `NV12_COL128`, its tiled 8- and 10-bit variants commented out of that list [21][22].

The BCM2711 peripherals datasheet, RP-008248-DS, is the likely source; this page does not cite it (not tested).

## Pre-Pi-4 boards

Nothing earlier than a Pi 4 is a candidate for a 4K artwork, and testing one is not worth the time (decided). Three independent lines of evidence agree.

- Every pre-BCM2711 product brief tops out at 1080p30 H.264 and MPEG-4 decode and never mentions 4K or HEVC [9][10][11].
- `bcm2835-codec` is the only hardware decode driver on those chips, and its 1920 px clamp applies [17]. HEVC is absent rather than limited: no entry in the driver's format table, and no HEVC node in `bcm2835.dtsi`, `bcm2836.dtsi`, `bcm2837.dtsi` or `bcm283x.dtsi` [18].
- A Raspberry Pi engineer wrote of the Pi 3 Model B+ that "The HW only supports up to 1080p60 …". The same engineer wrote that a successful software decode would be moot, because "the HDMI output is realistically limited to 1080P60" [43]. Decode ceiling and output ceiling both apply.

Software HEVC decode reaches only low-bitrate 1080p here. Community reports put a Pi 3 at roughly 11 Mbit/s of 1080p HEVC before it buffers, thermally limited; a second report gives roughly 15 Mbit/s at 100 % CPU [46]. LibreELEC caps the Pi 2 and Pi 3 at software standard definition in its 10.x releases, and a LibreELEC developer states that the optimised HEVC decode code from 9.2.x "was dropped in LE10 as it doesn't work with the new graphics stack (and, no, it won't come back)" [46].

No report of a 4K attempt on a Pi 0, Pi 1 or Pi 2 exists in this source set, so this page excludes them on the shared-silicon statements and the driver ceiling (assumed).

Display output is a separate claim from decode: the Pi 4 brief lists "2 × micro HDMI (up to 4Kp60 supported)" under video output and "H.265 (4Kp60 decode)" under multimedia, in different rows [12]. The earlier boards fail on both counts [43].

## Pi 5 H.264 decode

BCM2712 has no usable H.264 hardware decode or encode.

- `bcm2712.dtsi` carries the comment `/* IOMMU2 for … HEVC; and (unused) H264 accelerators */`. The tree exposes no H.264 node, binding or register range; the accelerator's only other trace is an unused `h264` clock-name string in the power-domain node, its clock reference commented out [20].
- A Raspberry Pi engineer states that BCM2712 has no H.264 hardware block for encoding or decoding [37].
- `bcm2835-codec`, the only driver that ever provided H.264 decode, binds through the firmware platform device instead of device tree, and BCM2712 does not run that firmware stack [17].
- The Pi 5 product brief has no multimedia row and never uses the string "H.264"; the Pi 500 brief lists only "H.265 (4Kp60 decode)" [13][14], where the earlier briefs all carry an explicit H.264 line [10][11][12].

BCM2711 also encodes H.264 at 1080p30 in hardware [7][12].

Official BCM2712 documentation leads with "4Kp60 HEVC hardware decode", follows with "Other CODECs run in software", and gives the CPU load of software H.264: roughly 10–20 % at 1080p24 and roughly 50–60 % at 1080p60 [8]. No vendor document frames the missing block as a regression; the absence is stated about BCM2712 alone [8][13][16].

A Raspberry Pi engineer says "The Pi5 can decode H.264 faster in software than the Pi4 can decode in hardware" [41]. That comparison carries no frame rate and no CPU figure, and no independent benchmark exists (not tested); the vendor's 50–60 % of the CPU at 1080p60 is the usable figure [8].

A third-party report measures the HEVC case on the same board: CPU use falls from roughly 45 % to roughly 20 % when an H.265 stream moves to `-hwaccel drm` on `/dev/video19`; no H.264 hardware transcode is available [47].

Two actions follow (decided). Measure before deploying an H.264 artwork on a Pi 5. Author anything new in HEVC, the one codec with a hardware path on both BCM2711 and BCM2712; a master transcoded to HEVC during [preparation](../guides/prepare-video.md) uses the Pi 5 decoder.

## Players and paths

| Player | BCM2835–2837B0 | BCM2711 | BCM2712 |
|---|---|---|---|
| `hello_video` (OpenMAX IL) | Works as a raw H.264 demo [27] | Works on the 32-bit legacy stack alone [28] | Fails; OpenMAX is unsupported on 64-bit kernels [35] |
| VLC | H.264 through V4L2 stateful [35] | One community report has HEVC hardware decode on bullseye (Debian 11), "presumably" bookworm (Debian 12) [38] | No direct evidence either way [40] |
| Kodi (v18 on MMAL and OpenMAX, v19 and later on V4L2) | Pi 1 has no HEVC; Pi 2 and Pi 3 are software and standard-definition in LibreELEC 10.x [29] | Hardware HEVC to 4K at 8, 10 and 12 bits, LibreELEC 10.x or later [29] | The same, LibreELEC 11.x or later [29] |
| ffmpeg | `h264_mmal` exists; one report has it failing with "Did not get output frame from MMAL" [51] | HEVC through V4L2 stateless in the OS-supplied build [38]; `hevc_v4l2m2m` implements the stateful API alone and cannot drive it [33] | HEVC through `-hwaccel drm` on `/dev/video19` [47] |
| mpv | H.264 through `v4l2m2m` expected, unverified by this project | The working realtime 4K30 HEVC path (measured on a Raspberry Pi 4) [1]; a third party observed `v4l2m2m` working [49] | HEVC works; frame drops are reported when the output path detiles [42] |
| dexd | Does not run | The realtime 4K30 configuration (measured on a Raspberry Pi 4) [1] | Not tested |

dexd's decode path is ffmpeg's V4L2-request HEVC decoder to DRM PRIME to a KMS plane, selected through mpv's `--gpu-hwdec-interop=drmprime-overlay` [1]; [Architecture](architecture.md) describes the stack.

One forum thread covers VLC on a Pi 5 [40]. In it, a reporter's working fallback was a custom player modelled on `hello_drmprime` [52], and the original poster's problem traced to UDP streaming. Treat VLC's Pi 5 decode path as unknown.

## Frame layout

On BCM2711 the HEVC decoder emits SAND-tiled NV12, which the display block scans out natively [21]. A display path that does not accept that layout detiles it first, and detiling costs most of the frame rate (measured on a Raspberry Pi 4) [1]. This project's own measurement and two third-party reports show frames lost that way.

- On a Pi 4 at 3840×2160, 30 fps: 14.3 fps for the CPU copy and about 5 fps through GL, against 29.1 fps on the zero-copy path [1].
- On a Pi 5 running trixie, mpv logged `VO: [gpu] 1920x1080 yuv420p`, converting away from the tiled buffer, and dropped frames in every output mode [42]:
  - 1080p60 output: 7 frames.
  - 4K60 output: 176 frames.
  - 3440×1440, over roughly 25-second clips: 500–760 frames.
- On a Pi 4 under Wayland, `--hwdec=v4l2m2m` produced a blue screen because Mesa could not import the format; `v4l2m2m-copy` worked at about 80 % of the CPU, with drops [49].

DRM allows a single authenticated master, and a Raspberry Pi engineer warns against running two independent processes to create overlays [34], so dexd uses mpv's native DRM output with no display server running. VLC, started fullscreen from X, borrows X's planes through DRM leases for zero-copy scanout [34]. Whether mpv can do the same is untested by this project.

dexd swaps mpv's plane defaults to keep the 4K video off the 3D render path; see [Architecture](architecture.md).

## Stateless decoder setup

On Raspberry Pi OS the stateless HEVC decoder may need `dtoverlay=rpivid-v4l2` in `/boot/firmware/config.txt`; the default has changed between releases, and LibreELEC enables it [30]. Adding the line takes effect after a reboot. These commands confirm the decoder either way:

```
sudo apt install v4l-utils
ls /dev/video*
v4l2-ctl --list-devices
```

`v4l2-ctl` lists each decoder under its driver name with its `/dev/videoN` nodes beneath; a `rpivid`, `rpi-hevc-dec` or `hevc_d` entry is a match. On the Raspberry Pi 4 used for the measurements that node is `/dev/video19` [1].

Without the overlay, ffmpeg fails with `No device available for decoder: device type drm needed for codec hevc` [50]. mpv instead falls back to software decoding and drops 4K frames, so dexd sets `hwdec-software-fallback=no`, which treats the fallback as an error.

ffmpeg needs `--enable-v4l2-request --enable-libdrm --enable-vout-drm` at build time for the arm64 V4L2-request path [50]. Raspberry Pi ships a downstream ffmpeg fork carrying those patches; upstream ffmpeg lacked full stateless support at the driver's kernel submission, and any later merge is unverified [23][33]. The fourth revision of the `rpi-hevc-dec` patch series reports 142 of 147 `v4l2-compliance` tests passing on kernel 6.16.0 [23].

## Pi 4 throughput

Measured on the Raspberry Pi 4 described in [the measurement record](measurements.md) [1].

Decode throughput on BCM2711 tracks megapixels per second, not the resolution label; 4K30 on this page is 3840×2160 at 30 fps. About 250 Mpix/s is the practical budget on the shipped zero-copy path, and a video above it will not play at its frame rate. ffmpeg's direct-to-DRM output goes higher, and dexd does not use it.

| Mode and path | Result |
|---|---|
| 4K30 HEVC, 249 Mpix/s, decode only | 1.36× realtime, roughly 41 fps |
| 4K40 HEVC, decode only | 1.08× realtime |
| 4K60 HEVC, 498 Mpix/s, decode only | 0.753× realtime, roughly 45 fps |
| 4K30 HEVC through mpv's zero-copy path | 0.969× realtime, zero dropped frames, about 35 % decode headroom |
| 4K40 HEVC through ffmpeg's direct-to-DRM output (see [vout_drm](../glossary.md#vout_drm)), not a dexd path | 1.70× realtime |
| 4K60 HEVC through mpv's zero-copy path | 0.976× realtime with 83 dropped frames |

At 4K60 the player stays close to realtime by discarding frames, which is why [the pass criteria](measurements.md) require a drop count beside the ratio. Only the display path blocks 4K40: the capture device used for the measurements advertises an HDMI 1.4 EDID, so it offers no mode above 2160p30.

The vendor specification for BCM2711 says "H.265 (4Kp60 decode)" [7][12][15]. That figure describes the decode block in isolation (assumed); a complete decode-and-present pipeline does not reach it: 0.753× realtime on a Raspberry Pi 4 [1]. An independent Kodi user reports 45 to 55 fps on a real 4K60 file, dropping to 25 fps, in a thread that reaches no resolution [45]. Do not plan a 4K60 artwork on a Pi 4.

### Display output

4K30 runs at a 297 MHz pixel clock, the HDMI 1.4 ceiling, so `hdmi_enable_4kp60` does nothing for it [1]. 4K60 needs 594 MHz and an HDMI 2.0 sink. On a Pi 4 it also needs `hdmi_enable_4kp60=1`, the first micro-HDMI port and one connected display, since driving both ports caps the board at 4K30 [1].

The vc4 driver builds modes from what the sink advertises and refuses one it does not. Two attempts on the capture device left the pixel clock at 297 MHz [1]:

- a forced mode, `video=HDMI-A-1:3840x2160@60` with `hdmi_enable_4kp60=1`;
- a request for a generated mode, `video=HDMI-A-1:3840x2160M@40`.

Lowering the frame rate does not lower the bandwidth at 4K, because the HDMI timing standard, CEA-861, gives 2160p24, 2160p25 and 2160p30 one 297 MHz clock and varies the horizontal blanking alone: total widths 4400, 5280 and 5500 px [1]. The mode negotiated for the measurements is 3840×2160, RGB 4:4:4, 8 bits per component, limited range, at that 297 MHz pixel clock [1].

### Bitrate

This project has measured no bitrate ceiling. One community sweep on a Pi 4 running LibreELEC reports 4K30 HEVC playback by bitrate [44]:

| Bitrate | Playback |
|---|---|
| to about 60 Mbit/s | smooth |
| around 70 Mbit/s | frames drop |
| above 80 Mbit/s | choppy on real-world rips |
| above 120–130 Mbit/s | a hard freeze |

The CPU stayed under 16 % throughout, and the sweep's author attributed the limit to HEVC intermediate memory and memory bandwidth [44]. It has not been reproduced; treat it as an order of magnitude. The project's own encodes played at realtime run at 25.3, 39.7 and 45 Mbit/s [1].

### Thermal limits

A Pi 4 soft-throttles at 80 °C, and throttling presents as intermittent frame drops [1]. Idle at 4K30 with no active cooling, the board reads 42.3 °C with no throttle bits set [1]. In a sealed passive case with no fan, playing a 2560×1440p60 video, the board holds about 77 °C, peaks at 78.4 °C and does not throttle over 2 hours 4 minutes — 1.6 °C under the limit [1]. A warmer room, a dusty enclosure or direct sun removes that margin, so an enclosure needs venting, a heatsink or a quiet fan (decided).

### Memory

The board reserves 512 MB of CMA for decoder frame buffers, unavailable to the player [1]. The player adds about 190 MB: a 123 MB video gave 314 MB of resident memory [1]. On a 1 GB board that leaves roughly 170 MB for the video: 1024 − 512 (CMA) − about 150 (kernel and userland) − 190 (overhead) [1].

Every figure comes from a 4 GB board: [the twenty-five-hour run](measurements.md) showed no leak but says nothing about fit on a smaller board. dexd sets no maximum video size; the board's memory is the bound, because the whole video is held in it.

## Pi 5 status

Everything about BCM2712 here is vendor- or community-sourced; this project has taken no Pi 5 measurements, and a Pi 4 result does not transfer to a Pi 5 (decided).

A Pi 5 decodes HEVC in hardware (assumed); the presentation path is unknown. Expect the failure already seen on a Pi 4: throughput looks realtime while each frame takes the slower, non-zero-copy route [1]. The mpv output and hardware-decode settings depend on what a Pi 5's mpv reports, so the Pi 5 test chooses them there.

Raspberry Pi's patched ffmpeg builds are suffixed `+rpt1` and `+rpt2`; trixie ships `+rpt1`, and whether a Pi 5 needs the `+rpt2` HEVC patches is unresolved. Settle it before a Pi 5 measurement.

## Legacy-stack playback

Gapless hardware playback on a Pi without dexd exists on Raspberry Pi OS buster alone, 32-bit, at 1080p30 (measured on the project's own buster image). It runs on `hello_video`, which needs the legacy Broadcom graphics stack that later releases dropped. `hello_video` does not start on bullseye, where `libbrcmGLESv2.so` is absent: `hello_video.bin: error while loading shared libraries: libbrcmGLESv2.so: cannot open shared object file` (measured).

Pinning an image to buster pins it to a 2019 Debian that can never be security-updated. Such an image suits an offline video installation and rules out anything network-exposed, a player with a web interface included.

A buster image boots on the Pi 4, the Zero 2 W and the original Zero, and not on the Pi 5. On the Pi 1 Model B it does not boot either, for reasons unresolved (measured on the project's buster image); Raspberry Pi Imager offers that board bullseye 32-bit as its newest release.

dexd needs a Pi 4 or later, so the cheaper boards drop out: about 18 Swiss francs for a Zero W and about 20 for a Zero 2 W, against about 66 for a Pi 5 (assumed: compared in 2024, no price source recorded).

## Board choice

| Board | Outcome |
|---|---|
| Pi 0, 1, 2, 3, Zero 2 W | 1080p H.264: the 1920 px clamp, no HEVC hardware, HDMI output limited to 1080p60 |
| Pi 4, 400, CM4 | 4K30 HEVC on the zero-copy path; 4K60 out of reach |
| Pi 5, 500, CM5 | HEVC in hardware, every other codec on the CPU; the display path unmeasured by this project |

## Measurement to-do

Each item below is unsourced or contradicted in the public record, and this page asserts none of them; the project's own test setup is the intended source.

1. Whether and when MMAL and OpenMAX were removed from Raspberry Pi OS, and whether `hello_video` still runs on bookworm; the bookworm release thread in this source set states no removal point [36].
2. The device node the legacy stateful decoder appears on.
3. Whether `h264_mmal` requires a `gpu_mem` firmware split (see [CMA](../glossary.md#cma)), and whether its output path copies.
4. Any H.264 level above 4.0 on pre-BCM2711 chips.
5. mpv's `v4l2m2m` path on pre-BCM2711 chips; the mpv documentation mirror in this source set carries no Raspberry Pi content [48].
6. Whether mpv can run a zero-copy KMS path under X11.
7. The HEVC bit-depth ceiling on BCM2711.
8. Whether ffplay supports the zero-copy KMS path on a Pi.
9. Upstream ffmpeg's V4L2-request merge status.
10. VLC's HEVC decode path on a Pi 5.
11. One Pi 5 forum thread reports the decoder advertising 1920 × 1088 while also claiming 4Kp60; the thread does not resolve it [39].
12. Capability rows for CM4, CM5 and the Pi 400, inferred here from shared-chip statements rather than their own briefs [7][8].
13. Per-model briefs for the Pi 2 Model B, Zero, Zero W, CM1, CM3, CM3+, the original Pi 3 Model B and the Pi 3 Model A+; the Pi 1 Model B+ brief is cited from a third-party mirror, since the vendor now serves only its mechanical drawings.
14. No public BCM2712 datasheet was located; the decoder block is Raspberry Pi's own design, so one may not exist.
15. An independent benchmark of Pi 4 H.264 hardware against Pi 5 H.264 software on one clip.
16. A direct 4K attempt on a Pi 0, Pi 1 or Pi 2.
17. A citation for the historical licence-key unlock of MPEG-2 and VC-1.

## Sources

Each entry was read from the fetched page, saved file or source file.

**This project's measurements**

1. dex project measurements, Raspberry Pi 4 and BCM2711, Debian trixie — see [the measurement record](measurements.md).

**Vendor documentation**

2. Raspberry Pi Documentation, Processors: BCM2835. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2835.adoc`
3. Raspberry Pi Documentation, Processors: BCM2836. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2836.adoc`
4. Raspberry Pi Documentation, Processors: BCM2837. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2837.adoc`
5. Raspberry Pi Documentation, Processors: BCM2837B0. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2837b0.adoc`
6. Raspberry Pi Documentation, Processors: RP3A0. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/rp3a0.adoc`
7. Raspberry Pi Documentation, Processors: BCM2711. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2711.adoc`
8. Raspberry Pi Documentation, Processors: BCM2712. `https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/processors/bcm2712.adoc`
9. Raspberry Pi Model B+ product brief, from a third-party mirror; the vendor no longer serves the original. `https://cdn-shop.adafruit.com/datasheets/pi-specs.pdf`
10. Raspberry Pi 3 Model B+ product brief, RP-008338-DS-2, published October 2025. `https://pip-assets.raspberrypi.com/categories/532-raspberry-pi-3-model-b/documents/RP-008338-DS-2-raspberry-pi-3-b-plus-product-brief.pdf`
11. Raspberry Pi Zero 2 W product brief, RP-008359-DS-1, published April 2024. `https://pip-assets.raspberrypi.com/categories/584-raspberry-pi-zero-2-w/documents/RP-008359-DS-1-raspberry-pi-zero-2-w-product-brief.pdf`
12. Raspberry Pi 4 Model B product brief, RP-008344-DS-5, published April 2026. `https://pip-assets.raspberrypi.com/categories/545-raspberry-pi-4-model-b/documents/RP-008344-DS-5-raspberry-pi-4-product-brief.pdf`
13. Raspberry Pi 5 product brief, RP-008348-DS-6, published April 2026. `https://pip-assets.raspberrypi.com/categories/892-raspberry-pi-5/documents/RP-008348-DS-6-raspberry-pi-5-product-brief.pdf`
14. Raspberry Pi 500 product brief, RP-008349-DS-4, published April 2026. `https://pip-assets.raspberrypi.com/categories/1115-raspberry-pi-500/documents/RP-008349-DS-4-raspberry-pi-500-product-brief.pdf`
15. Raspberry Pi 4 Model B specifications page, read from an Internet Archive snapshot. `https://www.raspberrypi.com/products/raspberry-pi-4-model-b/specifications/`
16. "Introducing: Raspberry Pi 5!", vendor news post. The line about removed H.264 hardware decoding in its comments is a reader's question, and no vendor statement. `https://www.raspberrypi.com/news/introducing-raspberry-pi-5/`

**Kernel and driver source**

17. `drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c`, raspberrypi/linux `rpi-6.6.y`. `https://github.com/raspberrypi/linux/blob/rpi-6.6.y/drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c`
18. `arch/arm/boot/dts/broadcom/` (`bcm2835.dtsi`, `bcm2836.dtsi`, `bcm2837.dtsi`, `bcm283x.dtsi`), raspberrypi/linux `rpi-6.6.y`. `https://github.com/raspberrypi/linux/tree/rpi-6.6.y/arch/arm/boot/dts/broadcom`
19. `bcm2711.dtsi`, raspberrypi/linux `rpi-6.12.y`. `https://github.com/raspberrypi/linux/blob/rpi-6.12.y/arch/arm/boot/dts/broadcom/bcm2711.dtsi`
20. `bcm2712.dtsi`, raspberrypi/linux `rpi-6.6.y`. `https://github.com/raspberrypi/linux/blob/rpi-6.6.y/arch/arm64/boot/dts/broadcom/bcm2712.dtsi`
21. `drivers/media/platform/raspberrypi/hevc_dec/hevc_d_video.c`, raspberrypi/linux `rpi-6.12.y`. `https://github.com/raspberrypi/linux/blob/rpi-6.12.y/drivers/media/platform/raspberrypi/hevc_dec/hevc_d_video.c`
22. `rpi-hevc-dec` upstream patch series v1, patchew archive. `https://patchew.org/linux/20241220-media-rpi-hevc-dec-v1-0-0ebcc04ed42e@raspberrypi.com/20241220-media-rpi-hevc-dec-v1-4-0ebcc04ed42e@raspberrypi.com/`
23. LWN.net mirror of the v4 patch cover letter. `https://lwn.net/Articles/1028029/`
24. "Raspberry Pi HEVC Decoder Driver Posted For Linux Kernel Review", Phoronix: one stateless V4L2 driver covering BCM2711 and BCM2712. `https://www.phoronix.com/news/Raspberry-Pi-HEVC-H265-Decode`
25. LWN.net mirror of the v5 patch cover letter. `https://lwn.net/Articles/1060711/`
26. "V4L2 HEVC driver", raspberrypi/linux pull request 3505, merged to `rpi-5.4.y`. `https://github.com/raspberrypi/linux/pull/3505/files`
27. `hello_video` source, raspberrypi/userland. `https://github.com/raspberrypi/userland/blob/master/host_applications/linux/apps/hello_pi/hello_video/video.c`
28. popcornmix/omxplayer project documentation. `https://github.com/popcornmix/omxplayer/blob/master/README.md`
29. LibreELEC documentation, Raspberry Pi hardware. `https://github.com/LibreELEC/documentation/blob/master/hardware/raspberry-pi.md`

**Raspberry Pi forums**

30. "All about accelerated video on the Raspberry Pi". `https://forums.raspberrypi.com/viewtopic.php?t=317511`
31. "H264 video decoding": a Raspberry Pi engineer on the level 4.0 specification. `https://forums.raspberrypi.com/viewtopic.php?t=298607`
32. "Should v4L2 enable HW decode of hevc/h265?". `https://forums.raspberrypi.com/viewtopic.php?t=296736`
33. "FFmpeg hardware acceleration for transcoding": a Raspberry Pi engineer on the downstream fork and on ffmpeg's stateful `*_v4l2m2m` decoders. `https://forums.raspberrypi.com/viewtopic.php?t=331026`
34. "Pi 4 h265 decoding display with overlay": the single-master constraint, and a Raspberry Pi engineer on DRM leases. `https://forums.raspberrypi.com/viewtopic.php?t=332651`
35. "OMXPlayer — why it's no longer there?": a Raspberry Pi engineer on OpenMAX, and "The underlying platform is V4L2." `https://forums.raspberrypi.com/viewtopic.php?t=346146`
36. "A little bit on RPiOS 'Bookworm'": 98 posts; it discusses no MMAL or OpenMAX removal. `https://forums.raspberrypi.com/viewtopic.php?t=352477`
37. "RPi5 Codec confusion": a Raspberry Pi engineer on the absent H.264 block, with community notes on VP9. `https://forums.raspberrypi.com/viewtopic.php?t=357870`
38. "HEVC Support on RPi 4/5": a community report on a Pi 4 under bullseye. `https://forums.raspberrypi.com/viewtopic.php?t=377567`
39. "Decoding H265 on Raspberry Pi 5 via V4L2": internally inconsistent on 1920 × 1088 against 4Kp60. `https://forums.raspberrypi.com/viewtopic.php?t=381601`
40. "Pi 5, VLC, HEVC playback": one community report. `https://forums.raspberrypi.com/viewtopic.php?t=390492`
41. "Raspberry Pi 5 and the Lack of Hardware H.264 Decoding": two Raspberry Pi engineers; the 13 posts give no frame rate and no CPU figure. `https://forums.raspberrypi.com/viewtopic.php?t=391283`
42. "Pi 5: mpv dropping frames": trixie and mpv, drop counts per output mode. `https://forums.raspberrypi.com/viewtopic.php?t=393942`
43. "4K videos on Rasp Pi 3 B+": Raspberry Pi engineers on the decode and output ceilings. `https://forums.raspberrypi.com/viewtopic.php?t=223801`

**Independent measurements and third-party reports**

44. "Pi 4 HEVC playback max bitrate", LibreELEC Forum: a single-user bitrate sweep, never independently reproduced. `https://forum.libreelec.tv/thread/22076-rpi4-hevc-playback-max-bitrate/`
45. "Raspberry Pi 4B not reaching 4K 60 fps when playing an HEVC movie", LibreELEC Forum: 45 to 55 fps, unresolved. `https://forum.libreelec.tv/thread/24403-raspberry-pi-4b-not-reaching-4k-60fps-when-playing-hevc-hdr-movie-9-97-1-fresh-i/`
46. "HEVC / 265 files on a Pi 3B, around 11Mbit, it can't keep up", LibreELEC Forum. `https://forum.libreelec.tv/thread/17577-hevc-265-files-on-a-pi-3b-around-11mbit-it-can-t-keep-up/`
47. "Raspberry Pi 5 H265 HEVC hardware decoding working", Frigate discussion. `https://github.com/blakeblackshear/frigate/discussions/18431`
48. "Hardware Decoding", a third-party mirror of mpv documentation, with no Raspberry Pi content. `https://mpv-player-mpv.mintlify.app/av/hardware-decoding`
49. mpv issue 10956: Pi 4 under Wayland. `https://github.com/mpv-player/mpv/issues/10956`
50. jellyfin-ffmpeg issue 129: the Pi 4 64-bit V4L2-request setup. `https://github.com/jellyfin/jellyfin-ffmpeg/issues/129`
51. "Using h264_mmal decoder on Raspberry Pi 4", ffmpeg-user list: an unresolved failure report. `https://www.mail-archive.com/ffmpeg-user@ffmpeg.org/msg23170.html`
52. jc-kynesim/hello_drmprime: a minimal DRM PRIME to KMS zero-copy reference. `https://github.com/jc-kynesim/hello_drmprime`
