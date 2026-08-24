# Architecture

dexd plays one HEVC video in an endless gapless loop on a Raspberry Pi 4. This page is for a developer reading the source for the first time: it describes the layers between the Rust code and the HDMI output, the route a decoded frame takes through them, and how the crate is divided. The division lets the decision logic be tested on a machine with no Raspberry Pi and no libmpv.

Terms are defined in [the glossary](../glossary.md). Measured numbers and their conditions are in the [measurement record](measurements.md); the looping mechanism itself is in [Why an endless stream](endless-stream.md).

## Technical stack

dexd is one process that links libmpv. It registers a stream protocol with the library, sets the playback options, issues the first `loadfile` and then services mpv's event queue for as long as the player runs. Everything below libmpv is FFmpeg, the kernel and the chip.

```
dexd          loop:// stream, mpv option set, startup checks,
              END_FILE as fatal, mpv log capture
libmpv 0.40   presentation and timing, DRM output, KMS plane import
FFmpeg        Annex-B demux, V4L2-request HEVC decode
kernel        rpi-hevc-dec -> dma-buf -> vc4 DRM/KMS -> plane, CRTC, connector
BCM2711       HEVC decode block -> CMA -> HVS -> PixelValve -> HDMI PHY
```

- dexd registers the `loop://` stream whose read callback never returns 0, so mpv never sees an end of file. It sets the option set below, runs the [startup checks](startup-checks.md) before mpv is created, treats an unexpected `END_FILE` as fatal, and forwards mpv's log messages to standard error.
- libmpv 0.40 does presentation and timing: scheduling locked to the display's refresh through `video-sync=display-resample`, `vo=gpu` with `gpu-context=drm` for modesetting, atomic commits and page flips, and `gpu-hwdec-interop=drmprime-overlay` to hand each decoded frame to a KMS plane.
- FFmpeg, inside mpv, demuxes the raw Annex-B stream and decodes it through the V4L2 request API. An elementary stream carries no timestamps, so the frame rate comes from the sidecar as `container-fps-override` (see [The sidecar check](sidecar.md)).
- The kernel connects the stateless HEVC decoder (`rpi-hevc-dec`, `/dev/video19`) to DRM/KMS through the `vc4` driver by dma-buf, landing on a plane, a CRTC and the configured connector.
- BCM2711 decodes into SAND-tiled NV12 in CMA. The HVS reads that layout without conversion and drives PixelValve to the HDMI PHY at 297 MHz TMDS for 3840×2160, 30 fps.

FFmpeg does the decoding, whichever player is used; mpv contributes presentation, timing and plane management. Debian trixie's ffmpeg already carries the Raspberry Pi HEVC patches, so hardware decode comes from system packages with no vendoring, and mpv is driven from a command line.

## Frame pipeline

A decoded frame does not travel up the stack. The HEVC block writes it once into CMA, and the HVS scans it out of that same memory. Between those two events only a dma-buf file descriptor moves, which libmpv hands to KMS as a framebuffer. Compressed bytes and control flow downward; pixels move at the bottom, in hardware.

```
software
  dexd ──► libmpv ──► FFmpeg demux ──► V4L2 decode request
                                              │
  libmpv ◄╌╌╌╌╌ dma-buf file descriptor ╌╌╌╌╌╌┘
     │
     └──► KMS atomic commit: the descriptor becomes a framebuffer on a plane
─────────────────────────────────────────────────────────────────────────────
hardware
  HEVC decode block ══► CMA ══► HVS ══► PixelValve ══► HDMI

  ──►  compressed bytes and control   ╌╌►  a descriptor, no pixels   ══►  pixels
```

## Zero-copy path

The Pi's HEVC decoder emits NV12 only in Broadcom's 128-byte-column SAND tiling; the driver refuses a request for linear NV12. The Pi 4 display plane scans SAND out natively, so a frame reaches the screen untouched — but only when it goes straight onto a KMS plane. Every other output arrangement converts the tiling first, and the conversion costs most of the frame rate.

mpv offers two ways of handing a DRM PRIME frame to the display — its interops — and only one of them avoids that conversion.

| Output arrangement | Result at 3840×2160, 30 fps |
|---|---|
| `gpu-hwdec-interop=drmprime-overlay` (frame onto a KMS plane) | realtime, 30 fps, no dropped frames |
| `drmprime` (mpv's import into an OpenGL texture) | about 5 fps |
| `hwdec=drm-copy` (detile on the CPU, not an interop) | 14.3 fps |

The cost is source pixels per second, the one quantity no encoder setting changes. Measured on a Raspberry Pi 4 running Debian trixie; conditions in the [measurement record](measurements.md).

| Change | Effect on playback |
|---|---|
| output resolution 3840×2160 → 2560×1440 | none |
| source resolution 3840×2160 → 1920×1080 | the rate doubled |
| bitrate 39.3 → 3.1 Mbps, a factor of 12.5, at the same resolution, frame rate and GOP | 14.3 → 15.2 fps, a gain of 6 % |

## Plane assignment

dexd puts the video on the primary plane and mpv's own drawing surface on the overlay plane, through `drm-drmprime-video-plane=primary` and `drm-draw-plane=overlay`. This is the reverse of mpv's defaults, and it keeps the 4K video off the 3D render path.

**Note:** mpv sets the plane stacking property on the video plane only, so whether the video stays visible under mpv's surface depends on the `vc4` driver's default plane ordering. The project verified the plane ordering on a Raspberry Pi 4; re-verify it after a kernel upgrade or on another DRM driver.

## Option set

dexd passes this option set to libmpv before `mpv_initialize`.

| Option | Value | Effect |
|---|---|---|
| `vo` | `gpu` | mpv's GPU video output |
| `hwdec` | `drm` | hardware decode, frames as DRM PRIME handles |
| `gpu-context` | `drm` | draw through DRM directly, with no compositor |
| `gpu-api` | `opengl` | the graphics API for mpv's own surface |
| `gpu-hwdec-interop` | `drmprime-overlay` | the frame goes onto a KMS plane |
| `drm-draw-plane` | `overlay` | mpv's surface on the overlay plane |
| `drm-drmprime-video-plane` | `primary` | video on the primary plane |
| `video-sync` | `display-resample` | schedule frames against the display's refresh |
| `hwdec-software-fallback` | `no` | a lost hardware path becomes an error |
| `fullscreen` | `yes` | fill the screen |
| `osc` | `no` | no on-screen controls |
| `input-default-bindings` | `no` | no key bindings |
| `terminal` | `no` | no terminal output; log messages arrive as events |
| `correct-pts` | `no` | mpv generates timestamps, since the stream has none |
| `demuxer-max-bytes` | `64MiB` | a ceiling on the read-ahead cache |
| `demuxer-readahead-secs` | `1.0` | the effective prefetch depth: one second of decoded-ahead insurance across the loop point |
| `container-fps-override` | the resolved frame rate | the frame rate the sidecar supplies |
| `drm-connector` | the resolved connector | always passed, so the output is never left to mpv's own pick |
| `drm-mode` | the resolved display mode | omitted when the display mode is `auto`, which is mpv's own default of `drm-mode=preferred` |

The last three values are resolved before mpv is created: the frame rate from the sidecar, the connector and the display mode from the [exhibit config](exhibit-config.md). `--no-defaults` drops the sixteen built-in options; it does not affect the last three.

The automated tests replace the option set with `--no-defaults --opt vo=null --opt vid=no --opt aid=no`, so mpv runs with no display and no hardware decode (see [Building and testing dexd](development.md)).

Without `hwdec-software-fallback=no`, a decoder that cannot reach the hardware path falls back to software without reporting it and plays 3840×2160, 30 fps at about 14 fps. With the option set, the same condition arrives as an `END_FILE`: dexd logs it and exits, and systemd restarts the player.

`demuxer-readahead-secs` is the bound that governs memory, because mpv runs its aggressive cache only for streams flagged as network and a custom stream is not one; `demuxer-max-bytes` is a second safeguard.

A rejected option exits 2: the same invocation fails identically on the next start, so a restart cannot help.

## Display ownership

DRM grants the right to drive a display to one process at a time. dexd's systemd unit conflicts `getty@tty1.service` away and orders itself after `multi-user.target`, which prevents “device busy” instead of recovering from it. The kernel releases DRM master when the process exits, so the next start acquires it cleanly. The settings are covered line by line in [The systemd unit](service-unit.md).

For the same reason the dex card is built on Raspberry Pi OS Lite. The desktop image ships a Wayland compositor, which would sit between the decoder and scanout.

**Note:** the single-master rule is not an absolute bar to a display server. A Raspberry Pi forum thread describes VLC, started fullscreen from X, borrowing X's planes through DRM leases and still scanning out with no copy (see [Raspberry Pi media capability](pi-capability.md)). Whether mpv can do the same is not tested by this project.

## Language and bindings

dexd is written in Rust. The player runs unattended for weeks at 3840×2160, 30 fps with a hard per-frame budget. Two failure classes common in C are costly under that load: a slow leak that appears only after days of uptime, and a use-after-free in buffer handling that shows as corrupt frames rather than a clean crash. Rust's ownership rules make the use-after-free a compile error, and free each buffer at the end of its scope, so no buffer's lifetime rests on a convention someone has to remember.

libmpv's `stream_cb` is a C API, so the Rust side of it is one `copy_nonoverlapping` against a per-frame deadline — no interpreter and no allocation between the bytes and mpv.

Only the libmpv entry points this program calls are declared, hand-transcribed from `mpv/client.h` and `mpv/stream_cb.h` of mpv 0.40.0. The surface is small and stable, and a build-time code generator would be a heavier dependency than the declarations it replaces on a device that builds offline. A transcription can rot, so `tests/ffi_constants.rs` asserts every event id and the one error code against the linked library through `mpv_event_name()` and `mpv_error_string()`, matching the strings character for character; that also catches a future renumbering.

Five further rules constrain the FFI declarations and callbacks in `src/main.rs`:

- `mpv_get_property_string`, `mpv_get_property` and `mpv_free` are not declared at all. A synchronous property read waits without a timeout for mpv's core thread to reach its dispatch loop, and a core stuck in a display call never reaches it.
- Only the first two fields of `mpv_event_end_file` are read; the trailing playlist fields are irrelevant to a single-file appliance, and reading a prefix of a `#[repr(C)]` struct is well defined.
- A property-change payload is a tagged union. The handler checks both the subscription tag and the format tag before it touches the data; the handler ignores a mismatch, and the next health check sees no new sample.
- Both the development and release profiles set `panic = "abort"`: unwinding across the FFI boundary, or out of `main` while mpv threads are live, is undefined behaviour.
- The binary sets `#![deny(unsafe_op_in_unsafe_fn)]`, so each unsafe operation needs its own `unsafe` block and its own safety comment, including inside an `unsafe fn`.

One portability detail is easy to undo by accident: the read callback converts its buffer with `.cast::<u8>()`, because `c_char` is signed on some hosts, macOS on aarch64 among them, and unsigned on the Raspberry Pi. A bare `buf` compiles only on the Raspberry Pi.

## Crate layout

The binary is a thin unsafe shell over the library. FFI declarations, the callbacks and the event loop live in `src/main.rs`; everything decidable lives in `src/lib.rs` and its modules, which carry `#![forbid(unsafe_code)]`. A forbid cannot be lifted locally, not even in a test module. The library builds and tests with `cargo test --lib` on any machine, with no libmpv and no display.

The library re-exports nine modules: `chunk` (loop-position arithmetic), `exhibit` (config grammar, display resolution, cmdline reconciliation, sysfs mode lists), `ffi_consts`, `health`, `heartbeat`, `nal`, `sha256`, `sidecar` and `watchdog`.

Policy lives in the library, wiring in the binary.

- `read_fn` performs the copy; `chunk::next_chunk` decides the offsets and enforces three rules: never answer with zero bytes, return the position to byte 0 as soon as a copy reaches the end of the payload, and saturate an oversized request.
- `health` decides whether a sample means progress, an in-place recovery or an exit; `main.rs` only feeds it position samples on a fixed cadence.
- `watchdog` builds the address, performs the handshake and wraps the non-blocking send, unit-tested against real sockets; the socket's lifetime and the call site stay in `main.rs`.
- `exhibit` holds the display, cmdline and sysfs functions, so they can be tested without a Raspberry Pi.

## Programs

The package installs two binaries, and the crate builds a third for use on a workstation.

| Program | Runs on | Privileges | Purpose |
|---|---|---|---|
| `dexd` | the player | unprivileged `dex` user | plays the video, see dexd(1) |
| `dex-exhibit-apply` | the player | root | writes the forced display mode into `cmdline.txt`, see dex-exhibit-apply(1) |
| `dex-sidecar` | a workstation | none | writes and checks a video's sidecar |

`dex-exhibit-apply` ships because an operator runs it on the device, as root, after editing the exhibit config; the player is sandboxed and writes no boot config. It is another thin privileged shell: the grammar and the reconciliation logic live in the library, testable with no root and no real boot partition. Its root test calls `geteuid` through a hand-written declaration instead of a dependency for one system call.

`dex-sidecar` is not installed: a video is prepared on a workstation and copied to the player (see [Prepare your video](../guides/prepare-video.md)). The package also ships `dex-wait-hdmi`, a shell script the unit runs before the player, see dex-wait-hdmi(1).

## Stream callback and memory

dexd reads the asset once at startup and leaks it as a `&'static [u8]`. That takes the filesystem off the hot path: no re-open, no page-cache dependency, no read stalling a frame at the loop point. It is never freed, because it must outlive every mpv thread. The files are small: a three-second video is 1.3 MB at 1920×1080 and 14.8 MB at 3840×2160.

The protocol's `user_data` is a leaked `Box` around the payload slice, not a pointer into `main`: mpv may dereference it from its own threads until `mpv_terminate_destroy` returns. A stack slot would be undefined behaviour the moment anything unwound. The `Box` is sixteen bytes and is never freed.

The per-stream cookie is a separate `Box<LoopStream>` that `open_fn` allocates and `close_fn` reclaims; mpv calls `close_fn` once.

Exclusive access in `read_fn` comes from mpv's stream layer, which drives one stream from one thread at a time. The open callback runs a seek probe before the read and close callbacks are installed, and close runs after demux teardown.

`cancel_fn` is left `None` so that nothing reaches the cookie from the cancel thread, which `stream_cb.h` documents as a second thread. Wiring it up would break the exclusivity described above; the cookie would need an atomic or a lock first. `assert_send::<LoopStream>()` fails the build if a future field makes the cookie non-`Send`, which the compiler cannot otherwise check across a raw pointer.

The open callback ignores the URL it is handed: the payload is fixed at startup, so there is nothing to parse.

Order matters at startup: `loop://` is registered before `mpv_initialize`, so the protocol exists when playback starts. The first `loadfile` uses the synchronous `mpv_command`, before anything can hang; every later `loadfile` from an in-place recovery uses `mpv_command_async`, so a stuck core cannot freeze the supervisor thread. The rest of that behaviour is in [Failure handling at runtime](failure-handling.md).

The event loop dispatches eight events and ignores the rest:

- `NONE` and `START_FILE` continue.
- `SHUTDOWN` breaks the loop and tears mpv down normally.
- dexd forwards `LOG_MESSAGE` to standard error as `mpv/{prefix}: {text}`.
- `PROPERTY_CHANGE` carries the `time-pos` sample the health check runs on. `COMMAND_REPLY` reports whether a recovery's `loadfile` was accepted: dexd logs a rejection and clears the one `END_FILE(reason=stop)` that attempt would have produced. dexd judges the recovery by whether `time-pos` advances again.
- `END_FILE` is fatal unless it is the stop from dexd's own recovery. `QUEUE_OVERFLOW` is fatal in every case: mpv drops events once its queue fills, and the dropped one may have been the `END_FILE`.

## Device layout

| Path or identity | Shape | Purpose |
|---|---|---|
| `dex` | system user, no login, no home, groups `video` and `render` | opens `/dev/dri`; everything else is denied by the unit's sandboxing |
| `/opt/dex` | root-owned, mode `0755`, mounted read-only into the unit | the video, its sidecar and the exhibit config |
| `/var/cache/dexd` | created by systemd, owned by the service user | mpv's shader cache, through `XDG_CACHE_HOME` |

`/opt/dex` is the assets directory. On the card the guides build it is a directory on the root filesystem; the unit waits for it as a mount point, so a card that gives the assets their own partition needs no change. See [Exhibit config](exhibit-config.md).

## Pass criteria

The architecture is judged on correct playback: realtime rate, full frame rate, correct colour and correct geometry, with a gapless loop as one clause of it. mpv's own counters do not settle it: on the slow interop paths mpv reports zero dropped, decoder-dropped and late frames while presenting every frame below realtime. Playback time against the wall clock is the measure that separates them (see the [measurement record](measurements.md)). A run that fails one of the other clauses says nothing about the loop point and is not counted as loop-point evidence.

dexd plays no audio. libmpv can play audio, so adding it later needs no change to this design; the roadmap is in [Roadmap and open questions](roadmap.md).

## Alternatives

Measured on a Raspberry Pi 4 at 3840×2160, 30 fps with one test video unless stated; the rows without a rate are failure modes. The playback-rate sampling script reads mpv on the plane path at 0.969× realtime; the script's own 0.2 s polling overhead is the assumed cause of the shortfall. The kernel vblank counter and mpv's own display-rate reading put that path at 30 fps with no dropped frames. Conditions in the [measurement record](measurements.md).

| Option | Outcome | Why not |
|---|---|---|
| ffmpeg `vout_drm` | 1.92× realtime, the fastest playback path measured | its author calls it non-production; held frames at the loop point, measured at 1920×1080, 60 fps because an HDMI capture device cannot resolve single frames at 3840×2160 |
| GStreamer `kmssink` | fails to bind a SAND dma-buf, falls back to copying into ordinary CPU-allocated buffers and runs out of memory | an upstream gap confirmed by a Raspberry Pi engineer, not a tuning problem |
| GStreamer `v4l2slh265dec ! glupload ! glimagesink` | 0.97× realtime | a GL import through GStreamer's own upload path, so no headroom; mpv's GL interop is far slower on the same hardware |
| VLC `drm_vout` | 0.91× realtime | logs a failure to set the atomic capability and leaves the atomic path |
| mpv `hwdec=drm` with `vo=drm` | selects the software decoder without reporting it | not the plane path at all |
| A purpose-built player | reference implementations of the decode-to-plane path exist (`hello_drmprime`) | it would reimplement mpv's timing layer and plane management, the two hardest parts of the job |
| Python | 0.6× realtime, frames held at random points across the loop, not at the loop point | the read callback cannot sustain about 5 MB/s under the interpreter lock |

The mpv choice is worth revisiting if its presentation path regresses on a future release, if a Raspberry Pi 5 needs a different interop, or if a held frame at the loop point appears that mpv cannot fix.
