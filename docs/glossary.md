# Glossary

Terms used in the dex documentation. Every entry was reviewed and approved by the project owner; changes go through a pull request that CODEOWNERS routes to him. Entries marked **user** are the only technical terms the user guides use without explanation; **developer** entries may appear in the design documents.

Writers: a term that is not in this list is either plain English or must be explained in the sentence that uses it. See `AGENTS.md` for the writing rules and the list of retired words.

## User-tier terms

### .265 file

A video file that holds only the compressed HEVC pictures, with no wrapper such as MP4 around them. dexd plays this format and nothing else, so every video is converted to a .265 file before it goes on the player.

Also written: raw HEVC stream · raw Annex-B file · .h265 · elementary-stream file

Tier: user

### .deb package

The installable package file for Debian-based systems such as Raspberry Pi OS. dexd is delivered as one .deb, installed with apt, which also pulls in the mpv library it needs and sets up the service that starts at boot.

Also written: Debian package · the .deb · dexd_<version>_arm64.deb

Tier: user

### asset

The one video file the player loops — the video asset — named by the `asset` line of the exhibit config, as a file name next to the config (for example `artwork.265`) or an absolute path. If no asset is named, dexd refuses to start rather than guess. The artwork is the whole installation the asset plays in.

Also written: video asset · the video · `asset` (config key)

Tier: user

### checksum

A short code computed from every byte of a file; change one byte and the code changes. The sidecar stores the video's checksum, so dexd can tell a stale, wrong or half-copied file from the prepared one, and refuses it.

Also written: SHA-256 · sha256 · hash · fingerprint

Tier: user

### cmdline.txt

The one-line file on the Raspberry Pi's boot partition that holds the start-up options for the operating system, including a forced display mode. dex-exhibit-apply edits it from the exhibit config; do not hand-edit it, and reboot after it changes.

Also written: /boot/firmware/cmdline.txt · kernel command line · boot options file

Tier: user

### connector

The name the operating system gives each physical video output; on a Raspberry Pi 4 the two HDMI ports are HDMI-A-1 and HDMI-A-2. The exhibit config names the connector the display is plugged into (default HDMI-A-1), and every display check uses it.

Also written: `connector` (config key) · HDMI-A-1 · HDMI-A-2 · HDMI port

Tier: user

### dex card

An SD card holding Raspberry Pi OS, dexd, the exhibit config and the video, which turns a Raspberry Pi into a player the moment it boots. Building one is the setup task; a spare card is the fastest repair at a venue.

Also written: player card · card · SD card · exhibition card

Tier: user

### dex-exhibit-apply

A helper command installed with dexd, run with sudo after editing the exhibit config. It writes the forced display mode from the config into cmdline.txt, changing nothing else, and prints REBOOT REQUIRED only when the file actually changed.

Also written: exhibit-apply · `sudo dex-exhibit-apply`

Tier: user

### dex-sidecar

The command that writes a video's sidecar (`dex-sidecar write`) and checks an existing one against its video (`dex-sidecar check`), run on a workstation before copying to the player. It uses dexd's own reader, so what passes here plays there.

Also written: dex-sidecar write · dex-sidecar check · sidecar-check (old name) · make-sidecar.sh (old script)

Tier: user

### dex-wait-hdmi

A helper that runs before dexd and waits, up to two minutes, for a display to report it is connected. Projectors can wake slower than the Raspberry Pi boots; without the wait the system picks a fallback resolution and never corrects it.

Also written: DEX_HDMI_TIMEOUT (its timeout setting)

Tier: user

### dexd

The player program: it plays one .265 video on a Raspberry Pi in an endless gapless loop, starts at boot as a system service, checks its own health and restarts itself. Package, command and service share the name; helpers keep the dex- prefix.

Also written: dex-loop (name during development) · dexd.service · the player

Tier: user

### display mode

The picture size and refresh rate the player asks the display for, written as WIDTHxHEIGHT@RATE (for example 3840x2160@30) or `auto`. Set it in the exhibit config to match the display; a mode the display cannot show makes dexd refuse to start.

Also written: `display_mode` (config key) · WxH@R · resolution and refresh · auto

Tier: user

### EDID

The information a display sends over the HDMI cable describing itself and the modes it can show. dexd and the Raspberry Pi rely on it to choose a mode; some displays send it late or wrongly, which is why kms_force and dex-wait-hdmi exist.

Also written: display identification · the display's self-description · edid-decode (tool that prints it)

Tier: user

### exhibit config

The one file on each player that says which video plays (`asset`), which display mode to use and which connector. It lives next to the video, as `/opt/dex/exhibit.yaml` or `exhibit.json`; the package installs none. dexd reads it at every start; the command line may cross-check it but never override it.

Also written: /opt/dex/exhibit.yaml · /opt/dex/exhibit.json · exhibit.yaml · exhibit.json · exhibit file · exhibit · the installation · venue setup · `venue` / `display` / `note` (informational config keys)

Tier: user

### exit codes

The number dexd returns when it stops. 2 means it refused to start because something in the setup is wrong (fix the config or files; a restart will not help); 1 means playback failed while running, and the service manager restarts it automatically.

Also written: exit-code contract · exit 1 · exit 2

Tier: user

### ffmpeg

A command-line tool that converts video between formats. The prepare-video guide uses it to encode HEVC with the settings dexd needs and to extract the .265 file; ffprobe, from the same toolkit, reports a file's size, frame rate and codec.

Also written: ffprobe (its inspection tool) · libx265 / x265 (its HEVC encoder) · hevc_mp4toannexb (its MP4-to-.265 filter)

Tier: user

### forced display mode

An exhibit-config setting (`kms_force`) that makes the Raspberry Pi output a fixed mode from boot (for example 3840x2160@30) instead of trusting what the display announces, or `none`. Needed for displays that announce 4K but never get it unforced; dex-exhibit-apply writes it into cmdline.txt.

Also written: `kms_force` (config key) · WxH@R / WxH@RD

Tier: user

### frame rate

How many pictures per second a video shows, for example 30 or 29.97 (written 30000/1001). A .265 file does not record it, so dexd takes it from the sidecar and refuses to start without one rather than play at the wrong speed.

Also written: fps · frames per second · `fps` (sidecar field) · `--fps`

Tier: user

### gapless

Playback that repeats with no visible break: no black frame, no held frame, no stutter between the last picture and the first. It is the property dexd exists to deliver, and what every measurement in the design record checks.

Also written: seamless · seamless loop · perfect loop · loops seamlessly

Tier: user

### hardware decoding

Turning compressed video back into pictures using a dedicated block in the chip instead of the main processor. A Raspberry Pi 4 can play 4K HEVC smoothly only this way, so dexd is built around it; software decoding is the slow fallback.

Also written: hardware decode · HW decode · hardware-accelerated video · hwdec (mpv's option for it)

Tier: user

### heartbeat

A status line dexd writes to the system log at start and every ten minutes: loop count (`loops=`), uptime, chip temperature, dropped and late frames, playback-position age, watchdog state. While it keeps coming the player is alive. It reports; the health check repairs; the watchdog restarts.

Also written: heartbeat line · `loops=` (was `wraps=`) · `temp=` · `frame-drops=` · `vo-delayed=` · `pos=` · `pos-age=` · `watchdog=`

Tier: user

### HEVC

The video compression format dexd plays, also called H.265. The Raspberry Pi 4 has a hardware decoder for it and for nothing newer, so 4K playback depends on the video being HEVC; other formats must be re-encoded first.

Also written: H.265 · High Efficiency Video Coding

Tier: user

### keyframe

A picture in a compressed video that is complete on its own, not described as changes from earlier pictures. A video for dexd must begin with one and must not let later pictures refer back across the start, or the loop cannot restart cleanly.

Also written: intra frame · I-frame · IDR (the exact HEVC term, developer glossary)

Tier: user

### loop point

The moment playback returns from the video's last picture to its first. Everything about a gapless loop is decided here: a pause, a held frame or a flash at the loop point is the defect dexd is designed to avoid and its measurements look for.

Also written: restart of the loop · wrap point (retired wording) · the wrap (retired wording) · seam (retired wording)

Tier: user

### mpv

The open-source media player whose engine dexd uses to decode and show video. dexd does not run the mpv program; it embeds mpv's library and feeds it the video, which is why installing dexd also installs the mpv library package (libmpv2).

Also written: mpv 0.40 (the verified version)

Tier: user

### one-file rule

Only one exhibit config may exist on a player: exhibit.json or exhibit.yaml, never both. If both are present dexd refuses to start and names both, so a venue never runs yesterday's settings from the file nobody edited.

Also written: exactly one exhibit config · the extension decides the parser

Tier: user

### Raspberry Pi Imager

The official program that writes Raspberry Pi OS onto an SD card and lets you set the hostname, user and SSH key before first boot. The player-card guide starts with it and sets the SSH key here.

Also written: Imager

Tier: user

### sidecar

A small text file next to the video, named like the video plus `.json`, holding its frame rate and checksum. dexd refuses to start unless it is present and matches, so a wrong frame rate or a half-copied video is caught before anything shows.

Also written: <video>.json · asset sidecar · sidecar file

Tier: user

### system log

The log on the player, read with `journalctl -u dexd`. It is the only place dexd reports why it refused to start or what went wrong while playing; on site a fault shows only as a black screen, so diagnosis starts here.

Also written: journal · systemd journal · journalctl · the log

Tier: user

### test card

A short synthetic video with a frame counter, rotating hands, colour bars and a checkerboard border, made to check that a player shows the picture correctly and loops gaplessly. The example-content package provides them; play one to check a new player card.

Also written: dex test card · test-card video · test video (user prose) · test content (user prose)

Tier: user

### video container

A file format such as MP4 or MOV that wraps video, audio and timing information together. dexd does not read containers: the video is taken out into a .265 file, which carries no timing, so the frame rate must be written to the sidecar.

Also written: container · MP4 · MKV · MOV · wrapper format

Tier: user

### watchdog

A timer kept by the service manager: dexd must report in every few seconds, and if it goes quiet for three minutes it is killed and restarted. It catches a player that is alive but stuck — the last of three layers after the health check and the heartbeat; the heartbeat shows `armed` when it is active.

Also written: systemd watchdog · WatchdogSec (its unit setting) · `watchdog=armed` · `watchdog=inert`

Tier: user

## Developer-tier terms

### --drm-mode

The mpv option through which dexd requests the display mode from the exhibit config. mpv matches it against the connector's modes by rounding the refresh to an integer, so decimal refresh values are refused in display_mode (see vrefresh).

Also written: mpv display-mode option · --drm-draw-plane · --drm-drmprime-video-plane

Tier: developer

### Annex-B

The byte-stream layout for H.264/HEVC in which each NAL unit (see NAL unit) is preceded by a start code (see start code) rather than a length prefix; a .265 file is exactly this. dexd's asset check scans it directly; the endless stream repeats its bytes.

Also written: raw Annex-B · Annex-B byte stream · start-code delimited stream

Tier: developer

### atomic commit

A KMS (see KMS) operation applying a set of display changes (mode, planes, buffers) in one step at the next screen refresh; a page flip only swaps the shown buffer. mpv drives the display this way; VLC's DRM output fell off it.

Also written: atomic modesetting · page flip · modeset

Tier: developer

### barcode

A row of black and white cells burned into every test-card frame encoding the frame number, plus two fixed cells to calibrate against. The capture side reads it back to detect dropped, repeated or reordered frames independent of picture content; every measurement rests on it.

Also written: frame-index barcode · sync cells

Tier: developer

### BCM2711

The Broadcom chip in the Raspberry Pi 4 (and 400/CM4). Its dedicated HEVC decoder and display path are what every dexd measurement was made on; capability tables key on the chip, not the board name.

Also written: Raspberry Pi 4 SoC · Broadcom BCM2711 · VideoCore VI

Tier: developer

### BCM2712

The Broadcom chip in the Raspberry Pi 5. It decodes HEVC 4K60 in hardware but has no H.264 hardware block, and its display pipeline differs from the BCM2711's; dexd's zero-copy path (see zero-copy path) is unmeasured on it.

Also written: Raspberry Pi 5 SoC · VideoCore VII

Tier: developer

### bench

The development workstation and the hardware around it — Raspberry Pis, a capture device, displays — where dexd is measured, as opposed to a deployed player. A single test setup on it is a test rig (see test rig). Measured numbers in these docs come from the bench, not from a venue.

Also written: the bench · test rig (a single setup on it)

Tier: developer

### build identity

The version and source commit compiled into dexd, printed as its first log line and by `--version`. `(nogit)` means the build could not learn its commit and is not official; `+dirty` means uncommitted changes; a journal that starts with an unidentifiable build is undebuggable later.

Also written: DEX_BUILD_ID · `(nogit)` · `+dirty` · .dex-build-id (stamp file) · version line

Tier: developer

### buster

Debian 10 (2019), the last release with the legacy Broadcom graphics stack that dex's old hello_video player needs. The old dexOS image is pinned to it, which caps output at 1080p and blocks upgrades; dexd targets trixie instead.

Also written: Debian 10 · Debian buster · buster (32-bit) · the buster limitation

Tier: developer

### capped VBR

An encoding setting: quality-targeted variable bitrate (CRF, see CRF) with a hard ceiling on peaks (maxrate/bufsize, see VBV). The Pi's decoder is bounded by peak demand, not average, so the ceiling is what keeps a busy scene from stalling.

Also written: CRF with maxrate/bufsize · quality-targeted with a ceiling

Tier: developer

### capture deficit

The known ~10 % gap between the frames the bench capture device should deliver at 4K and the frames it does; a property of the measurement instrument, not the player. Measurement records subtract it so it is not misread as dropped frames.

Also written: capture shortfall

Tier: developer

### cargo-deb

The Cargo subcommand that builds dexd's .deb from settings in Cargo.toml (dependencies, installed files, service unit). Pinned to its 2.x line because newer versions need a compiler newer than trixie ships.

Also written: cargo deb · [package.metadata.deb]

Tier: developer

### cargo-deny

A Cargo tool that checks the dependency tree against a policy file (deny.toml): security advisories, licence allow-list, banned crates. dexd's policy bans procedural-macro crates to keep the dependency set small and auditable, and CI fails on any violation.

Also written: deny.toml · cargo deny check · proc-macro ban

Tier: developer

### CC0-1.0

The public-domain dedication under which dex's content — test cards, video masters, branding — is released. Code is not under it (it disclaims patent grants, and Fedora disallows CC0 for code), which is why software uses MIT-0 (see MIT-0).

Also written: CC0 · public-domain dedication

Tier: developer

### CMA

A region of physically contiguous memory the Linux kernel reserves for hardware such as the video decoder. Decoded frames land here (512 MB on the test Pi), unavailable to ordinary programs; it is the memory the zero-copy path (see zero-copy path) never copies out of.

Also written: Contiguous Memory Allocator · gpu_mem (the older firmware split)

Tier: developer

### cmdline check

A startup check: dexd compares the exhibit config's kms_force with the video= entry (see video= token) in the running kernel's /proc/cmdline and refuses if they disagree, naming both repairs. An edited but unrebooted cmdline.txt fails here, on purpose.

Also written: cmdline gate · /proc/cmdline check

Tier: developer

### combined work

In GPL terms, a program that links a GPL library and must therefore be distributed under the GPL. dexd's source is MIT-0 (see MIT-0), but Debian's libmpv links GPL-3+ libsmbclient, so the shipped binary is a GPL-3+ combined work.

Also written: GPL combined work · why the .deb is GPL-3+

Tier: developer

### config drift

The failure where one copy of a setting is edited while the running system reads another — a display mode in cmdline.txt, a comment in config.txt and a service drop-in disagreeing. The exhibit config, the one-file rule and dex-exhibit-apply exist to end it.

Also written: stale config copy · config.txt comment block (the old place)

Tier: developer

### container-fps-override

The mpv option that fixes the frame rate for a stream that carries no timestamps; dexd passes the sidecar's frame-rate string to it verbatim, together with --no-correct-pts. Without both, mpv guesses a rate for a raw .265 stream and plays at the wrong speed.

Also written: --container-fps-override · --no-correct-pts

Tier: developer

### CRA

An HEVC keyframe type (NAL type 21) that starts an open GOP (see open GOP): pictures after it may still reference pictures before it. dexd refuses a video that starts with a CRA, because whether it loops cleanly would depend on the content.

Also written: Clean Random Access · CRA slice · open-GOP keyframe · IRAP (the family: BLA/IDR/CRA) · RASL (the leading pictures a CRA admits)

Tier: developer

### CRF

An x265/x264 quality-targeted encoding mode: bitrate varies to hold a chosen quality level. The prepare-video recipe uses it with a bitrate ceiling (see capped VBR) rather than a fixed bitrate.

Also written: constant rate factor · -crf

Tier: developer

### data partition

A FAT-formatted partition that any computer can open without extra software, holding the video, its sidecar and the exhibit config where dexd reads them, at `/opt/dex`, so a technician edits the config with the card in a laptop. Intended and not built: the card the guides build keeps `/opt/dex` on its root filesystem, reachable over the network. The dexOS image had the same idea.

Also written: dexdata · /dexdata · FAT partition · media partition

Tier: developer

### demuxer

The stage in mpv (via FFmpeg's libavformat) that reads the incoming bytes ahead of playback and splits them into frames for the decoder, on its own thread. dexd's read callback is called from it, and its read-ahead cache is bounded by --demuxer-max-bytes.

Also written: demux · demux thread · libavformat · --demuxer-max-bytes

Tier: developer

### dexOS

dex's own Raspberry Pi OS image, built with pi-gen (see pi-gen), which boots straight into the legacy hello_video player; still pinned to buster (see buster). "dexOS" is the brand, `dex-os` the repository name and lowercase id. The roadmap moves it to trixie with dexd as the player.

Also written: dex-os · packages/dex-os · Dexbian (old image name) · stage-dex (its custom build stage)

Tier: developer

### dpkg-shlibdeps

The Debian tool that reads which shared libraries a binary links and derives the package's Depends field from them; cargo-deb invokes it via `$auto`. It cannot see behavioural needs, so libmpv2 (>= 0.40.0) and adduser are declared explicitly next to it.

Also written: $auto · Depends: · sonames

Tier: developer

### DRM

The Linux kernel subsystem that owns graphics hardware and the display. dexd draws through it directly, with no desktop or compositor in between; only one process may hold it at a time (see DRM master). Not digital rights management.

Also written: Direct Rendering Manager · DRM/KMS · KMS/DRM

Tier: developer

### DRM master

The exclusive right to draw to a display through DRM (see DRM); one process holds it at a time. A text console (getty) or desktop session holding it blocks the player, so the service conflicts the console away and the image boots without a desktop.

Also written: display ownership · authenticated master · DRM lease · getty (the console that may hold it) · multi-user.target (the desktop-free boot target)

Tier: developer

### DRM PRIME

The kernel mechanism for sharing a memory buffer between devices by handle (a dma-buf file descriptor) instead of copying it. It is how the decoded frame travels from the HEVC decoder to the display plane in the zero-copy path (see zero-copy path).

Also written: dma-buf · DRM_PRIME · dma-buf handle

Tier: developer

### drmprime-overlay

The mpv setting that hands each decoded frame straight to a KMS plane (see KMS plane) with no copy and no GPU texture step. The only path measured to sustain 4K30 on a Pi 4; plain drmprime (GL import) and drm-copy run far slower.

Also written: --gpu-hwdec-interop=drmprime-overlay · overlay interop · HW-overlay mode · drmprime (GL import, the slow variant) · drm-copy (CPU copy, the slow variant) · detiling

Tier: developer

### drop counters

Two mpv properties: frames the decoder dropped, and frames the video output presented late. dexd learns them only from change events (see mpv_observe_property), accumulates them across recoveries and prints the totals in the heartbeat as frame-drops= and vo-delayed=.

Also written: frame-drop-count · vo-delayed-frame-count · frame-drops · vo-delayed

Tier: developer

### elementary stream

Compressed video on its own: a raw, container-less stream with no timestamps or metadata. dexd's asset is one; that is why the frame rate travels separately in the sidecar and why rotation metadata from a phone recording is lost on extraction.

Also written: raw stream · container-less stream · rotation matrix / display matrix (the metadata a container carries) · -noautorotate

Tier: developer

### `END_FILE` event

The mpv event `END_FILE`, saying playback of the current file ended, with a reason. dexd treats it as fatal (exit 1) except reason=stop arriving from its own in-place recovery, which it counts and absorbs; before that fix every recovery killed the process.

Also written: END_FILE · MPV_EVENT_END_FILE · END_FILE(reason=stop) · MPV_END_FILE_REASON_STOP · LOADING_FAILED / NOTHING_TO_PLAY (reasons)

Tier: developer

### endless stream

dexd's looping method: mpv is given the video as one stream that never ends, because the read callback jumps back to byte 0 instead of reporting end-of-file. The decoder never seeks or restarts, so the loop point costs nothing; every mpv loop option stalls there.

Also written: endless bitstream · the endless-stream design · read callback (read_fn) · loop-position arithmetic (next_chunk)

Tier: developer

### `EOF` signal

The signal a reader gives when a file has no more bytes; to mpv a read that returns 0 means final EOF and playback ends. dexd's stream callback never returns 0 — the endless-stream idea (see endless stream); an empty asset is an error, not EOF.

Also written: EOF · end of file · end-of-stream · short read (fewer bytes than asked, legal)

Tier: developer


Tier: developer

### example-content

The dex package holding the test cards (see test card): the After Effects sources, lossless masters and, per variant, an encode plus player-specific companion files. dexd's test videos come from it; how a card becomes a video is documented there, not in dexd.

Also written: packages/example-content · test-card package · dex-test-card-<dur>s-<res><fps>.mp4 (its naming pattern)

Tier: developer

### fail-closed

Refusing to proceed when a required input is missing or ambiguous, instead of guessing and running wrong; fail-open is the opposite, carrying on with a default. Every dexd startup check fails closed — no sidecar, no asset, two configs, a mode the display lacks — with a refusal that names the fix.

Also written: fail closed · fail-open (the opposite) · refuse rather than guess

Tier: developer

### FFI

Calling C code from Rust (here libmpv's C API) through hand-declared functions and constants; the calls are unsafe. dexd keeps it all in a thin shell in main.rs; the decision logic lives in a library that forbids unsafe code and is tested without mpv.

Also written: foreign function interface · extern "C" · FFI shell / pure core (the crate's split) · #![forbid(unsafe_code)]

Tier: developer

### frame-duration histogram

A tally of how many consecutive captures each source frame occupied when the capture runs faster than the content (see oversampling). A held frame at the loop point shows directly as a longer duration for the last frame; the record's stall numbers come from it.

Also written: dwell histogram (retired) · dwell counts · held_at

Tier: developer

### GOP

A run of compressed pictures that starts with a keyframe (see keyframe); the rest are stored as changes within the run, and its length is the keyframe interval. A closed GOP never references pictures outside itself. dexd requires closed GOPs with an IDR (see IDR) at byte 0, so the loop needs no seek.

Also written: group of pictures · keyint (its length) · closed group of pictures · no-open-gop=1 · keyint=min-keyint · scenecut=0 · closed GOP

Tier: developer

### health check

dexd's periodic check (about every ten seconds) that mpv's playback position (see time-pos) is still advancing, using only values mpv pushes as events. Two non-advancing samples in a row count as a stall and trigger an in-place recovery. First of three layers: the health check repairs, the heartbeat reports, the watchdog restarts.

Also written: tier-0 health check (retired wording) · stall detection · HealthMonitor / HealthAction (the types)

Tier: developer

### hello_drmprime

A small reference program showing the zero-copy decode-to-display path (V4L2 decoder to a KMS plane) on the Pi. Rejected in 2024 as not gapless, but it is the model a custom player would follow if mpv ever proved insufficient.

Also written: drmu

Tier: developer

### hello_video

Raspberry Pi's minimal demo player for the legacy 32-bit graphics stack; the only thing found in 2024 that looped H.264 gaplessly, so the old dexOS image is built on it. Plays raw H.264 only, needs buster (see buster); the known-good reference in dexd's measurements.

Also written: hello_video.bin · ilclient demo · the legacy player

Tier: developer

### HVS

The Broadcom display block on the Pi that composes planes and scans them out to the HDMI output; it reads the decoder's tiled frames directly (see SAND tiling). Its underrun counter is a useful health signal, and the Pi 5's differs from the Pi 4's.

Also written: Hardware Video Scaler · compositor block · PixelValve (the output stage after it)

Tier: developer

### idle mode

libmpv's default of staying alive and waiting after playback ends or fails instead of exiting. It caused dexd's first shipped bug — a failed load left the process alive with a black screen forever — which is why END_FILE (see END_FILE) is treated as fatal.

Also written: mpv idle · alive-but-black (the failure shape)

Tier: developer

### IDR

The HEVC keyframe type (NAL types 19 and 20) that fully resets the decoder: nothing after it refers to anything before it. dexd requires the video's first picture to be an IDR, so looping back to byte 0 is an ordinary keyframe, not a seek.

Also written: Instantaneous Decoder Refresh · IDR frame · IDR_W_RADL · IDR_N_LP

Tier: developer

### in-band / out-of-band

Two evidence channels in dexd's measurements: in-band is what the player reports about itself (mpv's drop counters, the heartbeat), free and always available; out-of-band is an independent capture of the HDMI output. Only out-of-band evidence proves the picture; in-band evidence flags where to look.

Also written: self-reported vs. captured · --dump-stats (mpv's in-band statistics)

Tier: developer

### in-place recovery

dexd's first response to a stall: re-issue `loadfile ... replace` (see loadfile replace) on its endless stream without exiting, restarting demux, decode and video output. Attempts come from a fixed budget (see recovery budget); when it is spent, the process exits and systemd restarts it.

Also written: tier-0 recovery (retired wording) · recovery · AttemptRecovery

Tier: developer

### ingest

Preparing a video for the player: encoding it as HEVC, extracting the .265 file and writing its sidecar. Done on a workstation, never on the player. User-facing text and the shipped messages say "prepare the video"; developer text may say ingest.

Also written: prepare a video · preparing the video (user-tier wording)

Tier: developer

### KMS

The part of DRM (see DRM) that sets the display's mode — resolution and refresh — and manages what is shown. If the display is not ready at boot, KMS picks a fallback mode and never corrects it — what kms_force and dex-wait-hdmi guard against.

Also written: Kernel Mode Setting · fkms (the Pi's legacy 'fake KMS' mode)

Tier: developer

### KMS plane

A hardware layer the display block composes on screen without a copy; a connector has a primary plane and overlays. dexd puts the video on the primary plane and mpv's drawing on an overlay (the plane swap), the only arrangement measured to sustain 4K30.

Also written: DRM plane · overlay plane · primary plane · plane swap · ZPOS (plane stacking property) · CRTC (the scanout timing object planes feed)

Tier: developer

### kmssink

GStreamer's KMS output element. On the Pi it cannot bind the decoder's tiled frames (a known upstream gap, confirmed by a Raspberry Pi engineer), falls back to CPU copies and runs out of memory at 4K — why GStreamer was ruled out rather than tuned.

Also written: GStreamer kmssink · GStreamer · glimagesink · v4l2slh265dec

Tier: developer

### libmpv

mpv's embeddable C library. dexd links it (version 0.40 is the verified floor), registers a custom stream through it, drives playback through its command and event API and reads its properties; the whole player is that library plus a small Rust shell.

Also written: libmpv2 (Debian package) · mpv client API · client.h · MPV_EVENT_* / MPV_ERROR_* (its constants)

Tier: developer

### lifecycle test

A CI job that installs the freshly built .deb in a clean trixie container, upgrades over it, removes and purges it, asserting the user, groups, files and service state at each step. It replaced piuparts, which has no build for the CI runner's architecture.

Also written: install/upgrade/purge test · piuparts (the tool it replaced)

Tier: developer

### lintian

Debian's package checker, run in CI so that any error or warning fails the build. The few tags dexd silences are listed in lintian-overrides with the reason for each, so a silenced check is a reviewable diff, not a hidden decision.

Also written: lintian-overrides · deploy/lintian-overrides · merged-usr (the layout behind one override) · ITP bug (behind another)

Tier: developer

### loadfile replace

The mpv command that opens a URL in place of what is playing. dexd re-issues it on the same loop:// URL (see loop://) as its in-place recovery, which creates a fresh stream and forces demux, decode and video output to re-initialise without restarting the process.

Also written: `loadfile ... replace` · reload · vo_reconfig (what it forces)

Tier: developer

### loop://

The URL scheme dexd registers with libmpv (via mpv_stream_cb_add_ro) so mpv reads the video through dexd's own read/seek/size/close callbacks instead of a file. The read callback wraps to byte 0 rather than reporting end-of-file, producing the endless stream (see endless stream).

Also written: loop:// protocol · loop:// stream · custom stream protocol · stream callback (stream_cb)

Tier: developer

### maintainer scripts

The scripts a Debian package runs as root at install, upgrade and removal (postinst, postrm). dexd's create the `dex` user and /opt/dex, warn about a stale service drop-in or a config naming no asset, and clean up on purge; they run under dash, not bash.

Also written: postinst · postrm · #DEBHELPER# · checkbashisms (the lint for them)

Tier: developer

### MIT-0

The licence dexd's source, packaging and docs are released under: MIT without the attribution clause, OSI-approved and GPL-compatible. Chosen over 0BSD as the more widely recognised drafting of the same intent; content uses CC0-1.0 (see CC0-1.0).

Also written: MIT No Attribution · 0BSD (the alternative considered)

Tier: developer

### Mpix/s

Decode throughput as pixels per second, a better predictor than a resolution label of whether a mode will hold frame rate. The capability record proposes about 250 Mpix/s as the practical Pi 4 HEVC budget (4K30 is 249).

Also written: megapixels per second

Tier: developer

### mpv core

mpv's internal thread that owns playback state, decoding and property values. If it hangs (say, in a display call to a dying projector) the process stays alive but gives no picture and no property updates: silence, not an error, which health check and watchdog notice.

Also written: core thread · the core · VO thread (its video-output thread)

Tier: developer

### mpv_command_async

The libmpv call that queues a command and returns at once; the reply arrives as an event. dexd issues its recovery `loadfile` this way, never through the blocking mpv_command (used once, at startup), so a stuck mpv core cannot freeze the event thread.

Also written: async command API · MPV_EVENT_COMMAND_REPLY · mpv_command (the blocking sibling)

Tier: developer

### MPV_EVENT_QUEUE_OVERFLOW

The mpv event meaning its internal event queue overflowed and events were dropped, possibly including an END_FILE. dexd treats it as fatal (exit 1) rather than continue on a queue whose contents it can no longer trust.

Also written: QUEUE_OVERFLOW · event ring overflow

Tier: developer

### mpv_get_property_string

The libmpv call that reads a property's value synchronously; if the mpv core (see mpv core) is hung it blocks forever, no timeout. dexd removed it from its FFI surface entirely (the heartbeat learns values from events) so no diagnostic can itself become the hang.

Also written: synchronous property read · blocking property read · mpv_free (its companion)

Tier: developer

### mpv_observe_property

The libmpv call that subscribes to a property so mpv delivers a MPV_EVENT_PROPERTY_CHANGE event whenever it changes; non-blocking by design. dexd registers time-pos (see time-pos) and the drop counters (see drop counters) this way at startup and never polls.

Also written: property observation · MPV_EVENT_PROPERTY_CHANGE · subscribe-then-events

Tier: developer

### mpv_wait_event

The libmpv call that waits, with a caller-supplied timeout, for the next event; dexd's supervisor thread (see supervisor thread) loops on it. Because it always returns by the timeout, the loop cannot block indefinitely inside mpv and safely hosts the health tick and watchdog ping.

Also written: event loop · mpv_terminate_destroy (the teardown call, avoided on the escape path)

Tier: developer

### MSRV

The oldest Rust compiler a crate promises to build with. dexd pins it to 1.85, the rustc Debian trixie ships, and CI builds with that same apt-installed compiler rather than rustup, so 'builds in CI' and 'builds on the device' are one claim.

Also written: minimum supported Rust version · rust-version (Cargo.toml field) · rustc 1.85 floor

Tier: developer

### NAL unit

The basic packet of an HEVC stream, each starting with a two-byte header whose type says what it carries: parameter sets, a keyframe slice, other slices. dexd's asset check reads only the leading NAL units — enough to know whether the loop can restart cleanly.

Also written: NAL · Network Abstraction Layer unit · nal_unit_type · leading NALs · VCL NAL (a slice-carrying unit)

Tier: developer

### noise floor

In dexd's measurements, the rate of capture glitches (dropped or repeated frames) seen mid-loop, from the same run and instrument. A loop-point defect counts only if its rate is statistically above this floor (see two-proportion test), so the instrument cannot fake a stutter.

Also written: capture noise floor · mid-loop baseline · anomaly rate (the quantity compared)

Tier: developer

### Norway problem

The classic YAML 1.1 pitfall where a bare `no`, `yes`, `on` or `off` silently becomes a boolean. It does not occur in dexd's YAML config because yaml-rust2 (see yaml-rust2) resolves scalars under the YAML 1.2 core schema; tests pin that behaviour.

Also written: YAML 1.1 boolean resolution · `no` → false · YAML 1.2 core schema (the fix)

Tier: developer

### NV12

A common YUV 4:2:0 pixel layout for decoded video. The Pi's HEVC decoder emits NV12 only in Broadcom's tiled variant (see SAND tiling), never linear NV12, which is why any path that wants ordinary pixels must convert first and loses frame rate doing it.

Also written: NV12_COL128 · NC12 · yuv420p (its planar cousin)

Tier: developer

### omxplayer

The Raspberry Pi's old hardware video player on the OpenMAX/MMAL firmware interface, removed from Raspberry Pi OS since bullseye. It leaves about 100 ms of black between plays, which is why the old dex image stripped it out and used hello_video.

Also written: OpenMAX IL player · MMAL (its firmware interface) · h264_mmal (ffmpeg's decoder on it)

Tier: developer

### open GOP

A GOP (see GOP) whose pictures may reference pictures before its first keyframe. Looping such a video would make the first frame depend on frames that are not there, so dexd's asset check refuses it even when the sidecar checksum matches.

Also written: CRA-led GOP · open group of pictures

Tier: developer

### oversampling

Capturing at a higher frame rate than the video (1080p60 capture of 30 fps content) so every source frame is sampled at least twice. It makes a held or dropped frame directly observable rather than statistically inferred; used wherever the capture device allowed.

Also written: 2x capture · decimation (the opposite: dropping every other frame to make 30 fps from 60)

Tier: developer

### parameter sets

Three header NAL units (types 32, 33, 34) an HEVC decoder needs before its first picture: video, sequence and picture parameter sets. dexd requires all three at the start of the file, ahead of the IDR (see IDR); the SPS may also carry timing information.

Also written: VPS/SPS/PPS · VPS · SPS · PPS · VUI timing_info (timing fields in the SPS)

Tier: developer

### pi-gen

Raspberry Pi's official tool for building OS images in stages. dexOS (see dexOS) is a pi-gen build with a custom stage and a small quilt (see quilt) patch series; it is also where GPL-licensed code enters the dex repository.

Also written: Raspberry Pi OS image builder · rpi-image-gen / sdm / CustomPiOS (alternatives named, not used)

Tier: developer

### pi_video_looper

Adafruit's Python video-looping framework for the Raspberry Pi, which loads a named player backend and provides USB copy-in and playlists. The old dexOS uses it with hello_video; the roadmap decides whether dexd becomes a backend for it or absorbs those features.

Also written: Adafruit_Video_Looper · video_looper · adafruit/pi_video_looper · video_looper.ini (its config)

Tier: developer

### pivid

A purpose-built gapless video player for the Pi (JSON config, REST API, aimed at installations), the strongest 2024 candidate before dexd. Its 32-bit builds failed and it has been dormant since; it was first on the pre-committed fallback list if mpv had failed.

Also written: egnor/pivid

Tier: developer

### positive control

In dexd's measurement method, a known-good case (the hello_video player on its own test asset) run through the same capture and analysis, proving the instrument does not invent defects; a planted-defect run proves it does not miss them.

Also written: known-good reference · controls (noise-floor / defect-injection / positive)

Tier: developer

### quilt

A tool that keeps changes to someone else's code as an ordered series of patch files applied on top of the pristine upstream. dex patches pi-gen this way (the series is the changelog) rather than forking; for pi_video_looper a fork was chosen instead.

Also written: patch series

Tier: developer

### realtime rate

How fast playback time advances against the wall clock, as a ratio; 1.0 is realtime and below 0.98 fails. Measured from mpv's own position, it is the first pass criterion — a player that decodes too slowly is not tested for gaplessness at all.

Also written: realtime · ratio ≥ 0.98 · steady ratio / overall ratio

Tier: developer

### recovery budget

The fixed number of in-place recoveries (see in-place recovery) dexd may attempt in its lifetime; it never refills. Once spent, the next stall makes the process exit so systemd restarts it — bounding how long a screen can stay wrong before the heavier remedy.

Also written: MAX_RECOVERY_ATTEMPTS · cumulative recovery attempts

Tier: developer

### required check

The one CI job GitHub branch protection requires: it runs always, depends on every other job and computes the verdict itself, so a skipped or path-filtered job cannot silently block or silently pass a merge (the trap a required check that never reports sets).

Also written: gate job · `required` (its display name) · required-checks gate

Tier: developer

### restart policy

The systemd unit settings that relaunch dexd two seconds after any exit and never give up (Restart=always with StartLimitIntervalSec=0 in [Unit] — in [Service] it is silently ignored). It is the second recovery step after in-place recovery; a reboot step is planned but not built.

Also written: Restart=always · StartLimitIntervalSec=0 · process restart · tier 1 (retired wording) · reboot escalation (planned; tier 2 retired wording)

Tier: developer

### REUSE

A specification and linter for machine-readable licensing: every file names its copyright and licence, using SPDX (see SPDX) identifiers. dexd's crate passes `reuse lint` in CI; the whole dex repository does not yet, because of GPL code inherited through pi-gen.

Also written: reuse lint · reuse.software · REUSE.toml · LICENSES/

Tier: developer

### rpivid

The Linux driver (successive names: rpivid, rpi-hevc-dec, hevc_d) for the Raspberry Pi's hardware HEVC decoder, exposed as a V4L2 stateless device (see V4L2). It emits only tiled NV12 (see SAND tiling); if it is missing, mpv silently falls back to software decoding.

Also written: rpi-hevc-dec · hevc_d · /dev/video19 · the HEVC decoder · dtoverlay=rpivid-v4l2 (the overlay that used to enable it)

Tier: developer

### SAND tiling

Broadcom's 128-byte-column layout in which the Pi's HEVC decoder writes frames. The display block scans it out natively, so a frame can go decoder to screen untouched; anything else must detile it on the CPU or GPU first, which costs most of the frame rate.

Also written: SAND · SAND-tiled NV12 · column-tiled · BROADCOM_SAND128 · detiling (the conversion out of it)

Tier: developer

### sandboxing

The systemd unit settings that confine dexd to reading one file: read-only system, no home, private /tmp, no new privileges, running as the unprivileged `dex` user. It is why boot-config writes live in a separate root-run tool (dex-exhibit-apply) instead of the player.

Also written: ProtectSystem=strict · NoNewPrivileges · PrivateTmp · XDG_CACHE_HOME=/var/cache/dexd (mpv's shader cache under it)

Tier: developer

### sd_notify

systemd's notification protocol: a service sends short messages such as WATCHDOG=1 to a socket systemd names in $NOTIFY_SOCKET. dexd implements the ping by hand — one non-blocking datagram, no libsystemd — and sends nothing else (no READY=1, no STOPPING=1).

Also written: NOTIFY_SOCKET · WATCHDOG=1 · WATCHDOG_USEC · WATCHDOG_PID · NotifyAccess=main · Type=simple (never Type=notify)

Tier: developer

### sidecar grammar

The restricted JSON a sidecar must follow: one flat object, string and unsigned-integer values only, no duplicate keys, standard escapes. Anything else is a parse error, and a parse error refuses startup — an unreadable sidecar and a missing one are the same fact.

Also written: strict subset · flat-object JSON subset · FpsSource (where the frame rate came from)

Tier: developer

### SoC

The single chip that holds a Raspberry Pi's processor, GPU and video blocks (BCM2711 in the Pi 4, BCM2712 in the Pi 5). Media capability is a property of the SoC, so capability tables and the heartbeat's temperature refer to it, not the board name.

Also written: system on chip · VideoCore IV / VI / VII (its GPU generations) · BCM2835 … BCM2712 (the part numbers)

Tier: developer

### SPDX

The standard short identifiers for licences (MIT-0, CC0-1.0, GPL-3.0-or-later) and the header lines that carry them in source files. REUSE (see REUSE) checks that every file in dexd carries them.

Also written: SPDX identifier · SPDX-License-Identifier

Tier: developer

### start code

The byte sequence 00 00 01 (or 00 00 00 01) that marks the start of every NAL unit (see NAL unit) in an Annex-B stream. dexd finds NAL units by a plain search for it; a stream with none is 'garbage' and is refused.

Also written: 00 00 01 · Annex-B start code

Tier: developer

### startup checks

The ordered refusals dexd runs before touching mpv: exhibit config parses; display mode resolved and cross-checked; cmdline agrees; mode exists on the connector; asset readable; sidecar parses; frame rate resolved; checksum matches; leading NAL units valid. Each failure names its fix and exits 2.

Also written: startup gates (retired wording) · gates · asset validation · NAL check · sidecar check · mode pre-flight · resolve_display / resolve_asset / resolve_fps (the decision tables)

Tier: developer

### stateless decoder

A hardware decoder driver model in which userspace parses the bitstream and hands the driver each frame's parameters (V4L2's request API). The Pi's HEVC decoder is stateless only, so ffmpeg's stateful hevc_v4l2m2m decoder cannot drive it; the working path is the V4L2-request hardware acceleration.

Also written: V4L2 stateless · request API · V4L2 Request API · v4l2request (mpv's hwdec name) · stateful / M2M (the other model)

Tier: developer

### supervisor thread

dexd's single thread that waits for mpv events (see mpv_wait_event), runs the health check, heartbeat and watchdog ping, and makes every mpv call. It must never block, so only non-blocking mpv calls are used on it; a hang here is what the systemd watchdog catches.

Also written: the event loop · event thread (retired wording)

Tier: developer

### sysfs

The Linux virtual filesystem under /sys that exposes kernel and device state as files. dexd reads a connector's status and mode list there for the mode pre-flight and dex-wait-hdmi, and the chip temperature for the heartbeat; tests point it at fixture files instead.

Also written: /sys/class/drm/card*-<connector>/modes · /sys/class/thermal/thermal_zone0/temp · modetest (the libdrm tool that lists the same modes)

Tier: developer

### test rig

One test setup on the bench (see bench): a Raspberry Pi, a display or capture device, and the flags that make dexd testable there. Flags prefixed `--test-rig-` skip the sidecar and exhibit config or force a failure on purpose; they warn loudly and never belong at a venue.

Also written: rig · testing rig · `--test-rig-*` flags · (test rig only)

Tier: developer

### thermal throttling

The Pi firmware slowing the chip when it runs too hot or its supply sags; `vcgencmd get_throttled` reports 0x0 when it has never happened. dexd's long-running tests record it alongside the heartbeat temperature, since a sealed enclosure can push a Pi 4 into it.

Also written: throttled=0x0 · vcgencmd get_throttled · under-voltage

Tier: developer

### time-pos

mpv's playback-position property in seconds. dexd subscribes to it once (see mpv_observe_property) and the health check judges progress only from its change events; the heartbeat prints the last value and how old it is (pos-age=), so staleness is visible.

Also written: playback position · pos= · pos-age=

Tier: developer

### TMDS

The electrical signalling HDMI uses; its character rate is the pixel clock. 4K30 needs 297 MHz, the ceiling of HDMI 1.4, which is why 4K30 works on older displays and capture devices while 4K60 does not; the kernel exposes the live rate as tmds_char_rate.

Also written: HDMI PHY · tmds_char_rate · 297 MHz · hdmi_enable_4kp60 (needed only above that)

Tier: developer

### trixie

Debian 13, the release Raspberry Pi OS is based on and the target for dexd's package. CI builds inside a debian:trixie container with Debian's own rustc 1.85 so the binary matches the device's libraries; its software HEVC decoder lets the recovery test run in CI.

Also written: Debian 13 · debian:trixie · Debian trixie · Raspberry Pi OS Lite (trixie)

Tier: developer

### two-proportion test

The statistical test dexd's measurements use to compare the glitch rate at loop points with the mid-loop noise floor (see noise floor); a run passes when the two are indistinguishable (p ≥ 0.01). It is why a verdict needs at least 500 loop points.

Also written: two-proportion z-test · statistical gate

Tier: developer

### USB copy mode

The way the old dex player takes new content: plug in a USB stick and it copies the files onto the card's data partition (see data partition), gated by a marker file. Roadmap: dexd's ingest (transcode to .265 plus sidecar) would happen during this copy.

Also written: usb_drive_copymode · copymode · USB copy-in · videopi (the marker file)

Tier: developer

### V4L2

The Linux kernel API for video devices, including hardware codecs; drivers are either stateful or stateless (see stateless decoder). The Pi's HEVC decoder appears as a V4L2 device (/dev/video19 on the test Pi), listed with `v4l2-ctl --list-devices`.

Also written: Video4Linux2 · v4l2-ctl

Tier: developer

### VBV

The encoder-side model that caps a stream's instantaneous bitrate through a virtual buffer, set with -maxrate and -bufsize. dexd's artwork encodes cap peaks at 45 Mbps, safely under what the Pi 4 decoder sustains and under the HEVC level ceiling.

Also written: Video Buffering Verifier · -maxrate / -bufsize · HEVC level / tier ceiling (Main@L5@High)

Tier: developer

### vc4

The Linux DRM/KMS driver for the Pi's display hardware (the sibling v3d driver handles 3D). It decides which modes to build for a connector — and built no 4K mode at all for one capture device unless forced, the case kms_force exists for.

Also written: vc4 driver · VC4 KMS driver · vc4-kms-v3d · v3d (the 3D sibling) · vc4.force_hotplug (rejected alternative to video=)

Tier: developer

### verdict

The four outcomes of one measured run: PASS (loop-point glitch rate indistinguishable from the noise floor), FAIL (significantly above it), VOID (below it, meaning loop points were misclassified — check the loop length), INSUFFICIENT (fewer than 500 loop points captured).

Also written: PASS / FAIL / VOID / INSUFFICIENT · run status · --loop-length (frames per loop, needed to classify)

Tier: developer

### video= token

The kernel command-line entry that forces a connector to a mode at boot; a trailing D also marks the connector as connected before anything is plugged in. dex-exhibit-apply writes it from kms_force, and dexd's cmdline check (see cmdline check) verifies the running kernel has it.

Also written: video=<connector>:<mode> · video=HDMI-A-1:3840x2160@30D · kernel mode force

Tier: developer

### vo=null

The mpv video-output setting that renders nothing. Every automated dexd test runs with it (plus software decoding, via dexd's --no-defaults), so CI proves the loop, the checks and the recovery logic but never the real DRM output; that part is verified on hardware.

Also written: --opt vo=null · --no-defaults (dexd flag that drops its Pi option set) · vid=no aid=no (the tests' deterministic-failure convention)

Tier: developer

### vout_drm

ffmpeg's experimental direct-to-DRM output. It was the fastest raw decoder measured (1.92× realtime) but loops worse than mpv and its author calls it non-production, so it stays a fallback note in the design record, not a shipped path.

Also written: ffmpeg -f vout_drm · drmu plane path · +rpt1 / +rpt2 (Raspberry Pi's ffmpeg build suffixes)

Tier: developer

### vrefresh

The integer refresh rate the kernel keys each display mode on. mpv matches --drm-mode by rounding to it, so a decimal such as 59.95 silently means 60; dexd therefore accepts only integer refresh values in display_mode and kms_force.

Also written: integer refresh · refresh-rate rounding

Tier: developer

### WPE WebKit

An embedded browser engine (WPE) and its minimal launcher (Cog); playing the test card as an HTML page through it was gapless for H.264 with occasional hiccups in 2024, the closest alternative to hello_video then. It appears only in the history of alternatives.

Also written: Cog · WPE · browser-based playback

Tier: developer

### yaml-rust2

The Rust YAML parser dexd uses for exhibit.yaml: a from-scratch port of libyaml with no procedural macros, resolving scalars near the YAML 1.2 core schema. Aliases and anchors are refused before load (they enable exponential expansion), and duplicate keys are errors.

Also written: billion laughs (the expansion attack refused) · serde_json (the JSON path's parser)

Tier: developer

### zero-copy path

The decode-to-display arrangement where the decoder writes the frame once and the display scans it out from that memory, only a handle moving (see DRM PRIME). On a Pi 4 the only path reaching realtime 4K30; mpv selects it with hwdec=drm, gpu-context=drm and drmprime-overlay.

Also written: zero-copy · hwdec=drm + gpu-context=drm + drmprime-overlay · HW-overlay mode

Tier: developer

