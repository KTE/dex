# Building and testing dexd

This page is for a developer with a checkout: which machine runs which command, how the tests are layered, and the rules that keep a test off a display in use.

## Two machines

Development uses a workstation and a Raspberry Pi, and neither alone is enough. The Raspberry Pi has what the player needs: DRM and KMS, the Broadcom HEVC decoder, and Debian's libmpv. A workstation without libmpv cannot link the `dexd` binary, so `cargo test` fails there while linking; `dex-sidecar` links neither libmpv nor a DRM device and builds anywhere.

Pure-logic tests run on either machine; anything that links libmpv runs on the Raspberry Pi or in CI. The workstation carries the capture and analysis side — see [Measurement record](measurements.md).

## Checkouts

Both machines have a checkout of the same repository and exchange work through the remote. On the Raspberry Pi, once:

```sh
git clone --no-recurse-submodules https://github.com/KTE/dex.git ~/dex
cd ~/dex/packages/dexd
```

The clone skips submodules because `packages/example-content` carries video masters a build does not need; it is then about 9 MB.

A checkout also gives the build its identity: `build.rs` reads the commit with `git rev-parse`, so the startup line names it — `dexd 0.1.0 (8b8c00ef5eef)`. See [Packaging](packaging.md#version-and-build-identity) for the other sources.

Encoded test videos are build products, not repository content: keep them outside the checkout, for example under `~/assets/`.

**Note:** a test that must outlive the SSH session that started it needs `loginctl enable-linger <user>`; without it, systemd ends the user's session at the last logout and kills its processes.

## Toolchain

The Raspberry Pi builds with Debian's compiler from apt: trixie ships Rust 1.85, and `Cargo.toml` declares `rust-version = "1.85"`, which states the limit. Compiling with that toolchain, on the device and in CI, is what enforces it, so a dependency that needs a newer compiler cannot be adopted. A deployed Raspberry Pi is also a build host, so raise that floor only after confirming the devices can still build the package.

On the Raspberry Pi (and in the CI container):

```sh
sudo apt install build-essential pkg-config rustc cargo rust-clippy libmpv-dev ffmpeg
```

`libmpv-dev` is what the binary links; `ffmpeg` supplies the `ffmpeg` and `ffprobe` commands `tests/sidecar_write.rs` uses to make real HEVC streams. Building the .deb also needs `dpkg-dev`, `lintian` and cargo-deb from crates.io — see [Packaging](packaging.md).

The workstation needs only a Rust toolchain at 1.85 or newer — usually rustup on macOS — because nothing it builds links libmpv or ships in the package.

Everything else comes from cargo: four direct dependencies — `serde`, `serde_json`, `sha2` and `yaml-rust2` — plus their transitive closure in `Cargo.lock`.

## Commands

Run every command below in `packages/dexd`; there is no workspace root.

| Machine | Command | What it covers |
|---|---|---|
| workstation | `cargo check --all-targets` | type-checks every target, including unlinkable ones |
| workstation | `cargo test --lib` | the library's tests |
| workstation | `cargo test --bin dex-sidecar` | the sidecar writer's own tests; it links no libmpv |
| workstation | `cargo build --release --bin dex-sidecar` | the tool that prepares a video, which the package does not install |
| Raspberry Pi | `cargo test` | library, binary target and the three integration targets |
| Raspberry Pi | `nice -n 19 cargo test` | the same, kept off the CPU of a long-running test |
| Raspberry Pi | `cargo build --release` | the release binaries, including the two the package installs |

## Test layers

- Library tests cover the pure logic — `chunk`, `exhibit`, `ffi_consts`, `health`, `heartbeat`, `nal`, `sha256`, `sidecar` and `watchdog` — with no libmpv and no display, so `cargo test --lib` runs anywhere. The crate is cut so that everything decidable lives there — see [Architecture](architecture.md).
- Binary-target tests in `src/main.rs` call no mpv function but link libmpv, so they run on the Raspberry Pi; `cargo check --all-targets` type-checks them elsewhere.
- Integration tests spawn the binary or link the library: `tests/cli.rs` for failure paths and exit codes, `tests/ffi_constants.rs` for the hand-transcribed constants against the linked library, `tests/sidecar_write.rs` for `dex-sidecar write` against ffmpeg-made streams.

Every test in `tests/cli.rs` but one asserts exit behaviour — the exit code, and that the process ended at all — because failure paths are where this program's defects occur.

Two tests cover garbage input. `garbage_bytes_refused_at_startup_exit_2` proves the asset check refuses it, exit 2 and inside the deadline. `playback_failure_exits_nonzero_never_hangs` forces a failure after that check with `--opt vid=no`, so the process still has to exit once mpv is running.

`DEXD_ALLOW_MEDIA_SKIP=1` skips the sidecar-writer tests where ffmpeg cannot make a stream; skipping is opt-in because a skipped test still reports a pass.

## Display safety

A Raspberry Pi under test may be showing something. All but one of the invocations in `tests/cli.rs` that can reach mpv creation carry:

```
--no-defaults --opt vo=null --opt vid=no --opt aid=no
```

The null video output never touches DRM, and deselecting every track makes mpv end deterministically (`NOTHING_TO_PLAY` → `END_FILE`) instead of playing on.

One test departs from the rule. `force_recovery_survives_against_real_mpv` keeps the video track selected to observe a health check against playback that is advancing; `vo=null` keeps it headless, and its own time limit bounds the decode. It carries `#[ignore]`, so no plain `cargo test` starts it:

```sh
cargo test --test cli force_recovery_survives_against_real_mpv -- --ignored --nocapture
```

Run it on a Raspberry Pi with nothing else on the display and no long-running test in progress. CI runs it by exact name in a step of its own.

## Test harness

`tests/cli.rs` spawns the binary found at compile time through `env!("CARGO_BIN_EXE_dexd")`, with stdout discarded and stderr piped; a thread drains that pipe, because a child that filled it would block and look like a hang.

`run_with_deadline` polls the child every 50 ms and kills it on overrun. The deadline is what catches a player that hangs on a failure path instead of exiting.

Three outcomes stay distinct: exited with a code, killed at the deadline, or died by signal.

Scratch files go to `dexd-test-<pid>-<name>` in the system temporary directory, so parallel runs cannot collide.

## Fixtures

The HEVC fixture is a real stream: `stub_annexb()` returns the NAL units of a single-frame encode, parameter sets and one IDR slice, produced by:

```sh
ffmpeg -f lavfi -i color=c=black:s=16x16:d=1:r=1 -frames:v 1 -c:v libx265 -x265-params keyint=1 -f hevc frame.265
```

`loop://` never returns end of file, and the demuxer's probe gives up early only when it reaches one. Fed garbage, the probe extracts nothing and keeps requesting data — a full CPU core, memory growing, no output — until the deadline kills the run (measured on a Raspberry Pi). Real parameter sets let it resolve width, height and profile in one pass, so mpv reaches `END_FILE` in well under a second.

## Test-only flags

`--proc-cmdline PATH` supplies a synthetic kernel command line, so a test does not depend on the host's `/proc/cmdline`, which differs by machine. It is absent from the usage text: a deployment reads the real file.

`--test-rig-force-recovery-after-secs` and `--test-rig-hang-after-secs` force a failure on purpose; each requires `--test-rig-no-sidecar --fps <F>`, and [Failure handling](failure-handling.md) describes them.

To see the systemd watchdog fire, run `--test-rig-hang-after-secs 0` under a throwaway unit with a short window. The recipe needs the package installed, an asset at `/opt/dex/artwork.265`, and the `dex` user and group the postinst creates:

```sh
sudo systemd-run --unit=hang-test -p Type=simple -p NotifyAccess=main \
  -p WatchdogSec=15 -p Restart=on-failure -p RestartSec=2 \
  -p User=dex -p Group=dex -p SupplementaryGroups=video \
  /usr/bin/dexd /opt/dex/artwork.265 --test-rig-no-sidecar --fps 30 \
  --test-rig-hang-after-secs 0 --no-defaults --opt vo=null --opt vid=no --opt aid=no
```

`journalctl -u hang-test` then shows the watchdog timeout, the `SIGABRT` and the restart; the results are in the [measurement record](measurements.md).

## Lints

Four checks make up the lint standard:

- `cargo clippy --all-targets -- -D warnings`, with Debian's clippy in the CI container deciding, because findings differ between clippy builds and platforms;
- `#![forbid(unsafe_code)]` in the library, so no module and no test module can lift it;
- `#![deny(unsafe_op_in_unsafe_fn)]` in `src/main.rs`, so every unsafe operation is scoped where it happens;
- `cargo deny check` for the dependency policy — see [Packaging](packaging.md).

Miri is a follow-up: it cannot cross the FFI boundary, so its scope is the library.

Documentation and commit conventions, and the `scripts/docs-lint.mjs` gate that enforces them, are in [`AGENTS.md`](../../AGENTS.md).

## Before a change lands

```sh
cargo clippy --all-targets -- -D warnings
cargo test                   # on a Raspberry Pi; cargo test --lib on a workstation without libmpv
cargo deny check
node scripts/docs-lint.mjs   # from the top of the repository
```

A pull request runs these and the rest of the workflow, and one required check
reads their results; [Continuous integration](ci.md) says which jobs run when
and what each proves.

## Regression proof

Prove a regression test bites: re-introduce the defect, watch the test fail, then revert:

- change `MPV_EVENT_LOG_MESSAGE` in `src/ffi_consts.rs` from 2 to 6 — `event_ids_match_the_live_library` fails on the Raspberry Pi;
- replace the `END_FILE` branch body in `src/main.rs` with `continue` — `playback_failure_exits_nonzero_never_hangs` fails at its deadline, after about 30 seconds.

New work follows the same order: write the failing test, watch the assertion fail, implement the minimum that makes it pass, commit.

## Portability details

`c_char` is signed on macOS and unsigned on the Raspberry Pi. `read_fn` in `src/main.rs`, the stream read callback mpv calls for data, uses `buf.cast::<u8>()` rather than `buf as *mut u8`, which is a real conversion on macOS and an unnecessary cast clippy rejects on the Raspberry Pi; a bare `buf` compiles only on the Raspberry Pi.

An `AF_UNIX` path is capped at the size of `sockaddr_un.sun_path` — 104 bytes on macOS, 108 on Linux. The macOS temporary directory alone comes close enough to produce `EINVAL`, so the watchdog's socket tests bind under `/tmp`, with a short fixed prefix and a per-process counter keeping every path inside the budget.

## Alternatives

| Option | Outcome |
|---|---|
| Copying the crate to the Raspberry Pi instead of cloning it | Rejected: without `.git` the build cannot name its commit, and a separate identity file has to be kept in step |
| rustup for the package build (CI container and device) | Rejected: "builds here" and "builds on a device" become two claims that drift — see [Continuous integration](ci.md) |
