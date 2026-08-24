# Packaging

dexd is delivered as one Debian package, `dexd_<version>_arm64.deb`, built in CI and installed with apt. This page describes what the package contains and the rules a change to it must keep: install paths, the declared library floor, the crate policy, the toolchain floor, version and build identity, and the licence split. It is for a developer editing `Cargo.toml`, `deny.toml` or the files under `deploy/`.

Terms are defined in [the glossary](../glossary.md). The workflow that builds and checks the package is described in [continuous integration](ci.md).

## Delivery

The player installs a built artifact and compiles nothing on the device.

The package states the library requirement, so a device whose libmpv is too old for dexd fails at install, in front of whoever runs apt, instead of showing a black screen at the venue. The package also sets the install paths and creates the `dex` user and `/opt/dex`, so no install depends on the person who imaged the [dex card](../guides/install-dexos.md) remembering them.

CI builds on an arm64 runner inside a `debian:trixie` container, so the binary links the same libmpv the devices carry; another distribution's libmpv would produce the mismatch the package exists to catch.

The crate lives in the dex repository at `packages/dexd`, not as a submodule, so a change to the code, the packaging, CI and the dexOS image is one commit.

Each build's .deb is copied to a device and installed from that file. An apt repository is planned; see the [roadmap](roadmap.md).

## Contents

The package installs three programs, their documentation and the service unit.

| Path | Content |
|---|---|
| `/usr/bin/dexd` | the player, mode 755 |
| `/usr/bin/dex-exhibit-apply` | applies a changed exhibit config, mode 755 |
| `/usr/bin/dex-wait-hdmi` | waits for a connected display, mode 755 |
| `/usr/share/man/man1/` | `dexd.1`, `dex-exhibit-apply.1`, `dex-wait-hdmi.1` |
| `/usr/share/doc/dexd/` | `README.md`, the copyright file, the changelog |
| `/usr/share/lintian/overrides/dexd` | the override file and its reasons |
| `/usr/lib/systemd/system/dexd.service` | the unit, enabled at install and left stopped |

Binaries go to `/usr/bin`, never `/usr/local/bin`: Debian policy reserves `/usr/local` for the local administrator.

`dex-exhibit-apply(1)` ships because it runs on the device, as root, whenever the exhibit config changes. `dex-sidecar(1)` does not ship: it writes and checks a sidecar, and a video is prepared on a workstation.

The package installs no video, no sidecar and no exhibit config, and creates no `/etc/dex`. All three are content that changes per installation and live together in `/opt/dex`, the assets directory; `postinst` creates it on the root filesystem. A missing config makes dexd refuse to start and name the file to create — see [exhibit config](exhibit-config.md).

The unit is enabled at install (`enable = true`) and left stopped (`start = false`). A device that is only power-cycled comes back playing, and a technician chooses the moment the player takes DRM master. [The systemd unit](service-unit.md) covers its settings.

The release profile sets `opt-level = 2`, `lto = true` and `panic = "abort"`; the dev profile aborts on panic as well, so a panic behaves the same way under test. Cargo discovers `src/lib.rs` and `src/main.rs` on its own, so `Cargo.toml` carries no target sections, and `cargo test` ignores `panic = "abort"` for test builds.

## Depends

`Depends` has two halves: one derived from the built binary, one stated in `Cargo.toml`.

```toml
depends = "$auto, libmpv2 (>= 0.40.0), adduser"
```

Two more fields state a relationship no symbol expresses:

```toml
conflicts = "dex-loop"
replaces = "dex-loop"
```

dexd was packaged as `dex-loop` before it was named, and the two ship four of the same files — the `dex-exhibit-apply` and `dex-wait-hdmi` commands and their man pages. Undeclared, dpkg stops on the first of them and leaves a half-installed player on a card that still carries the older package. `Conflicts` states the pair is exclusive and `Replaces` lets apt do the rename in one step. CI reads both fields back out of the built package, because cargo-deb drops a manifest key it does not support without an error.

`$auto` runs dpkg-shlibdeps over the built binary, so the shared-library half of `Depends` follows the sonames the binary links and nobody writes it by hand. `ldd` reports 228 shared objects for the binary (measured; see [measurements.md](measurements.md#packaging-checks-on-hardware)); linking libmpv accounts for that count. dpkg-shlibdeps names only the packages providing the sonames the binary links itself, so a build resolves the derived half to a line such as `Depends: libc6 (>= 2.34), libmpv2 (>= 0.40.0)`.

From symbols alone the floor is `libmpv2 (>= 0.19.0)`, the oldest libmpv exporting the symbols dexd calls. Two things dexd relies on are behaviour, which dpkg-shlibdeps cannot see:

- the option `--gpu-hwdec-interop=drmprime-overlay`;
- the `END_FILE` event with `reason=stop` after a `loadfile replace`, which [in-place recovery](failure-handling.md) waits for.

Both hold in libmpv 0.40, the version trixie ships. A device with 0.19 installs the package cleanly and then plays wrong, so the stated floor sits next to `$auto`. CI asserts the derived half and the stated floor separately, because deleting the stated floor still yields a package that builds and installs.

Raise the floor whenever a fix is verified against a newer mpv.

`Cargo.toml` declares `adduser` because `postinst` calls it, and lintian fails a maintainer script that uses a tool the package does not depend on.

Installing the package pulls in more than these lines: libmpv2's own libraries, the two programs libmpv2 recommends (aria2 and yt-dlp), and systemd and dbus because a unit ships.

## Crates

`Cargo.toml` declares four crates, none of them a procedural macro, for about two dozen crates in the lock file.

| Crate | What it does |
|---|---|
| `serde` | the map visitor only, with no `derive` feature; the sidecar grammar makes a duplicate key an error, which `serde_json`'s last-wins map cannot express |
| `serde_json` | parses the sidecar, including `\uXXXX` escapes and UTF-16 surrogate pairs |
| `sha2` | SHA-256 for the asset checksum |
| `yaml-rust2` | parses the `.yaml` exhibit config and errors on a duplicate mapping key |

Add a crate only when it removes code dexd would otherwise carry, and only when its dependency tree is small enough to read. `Cargo.toml` declares `yaml-rust2` with `default-features = false`, which drops the `encoding` feature and `encoding_rs`, because the config is read with `fs::read_to_string` and so is UTF-8 already; that brings its measured cost to five added crates. The other YAML parsers weighed against it are in the alternatives table.

The [watchdog](failure-handling.md) ping adds no crate and no entry to the derived `Depends`: sending `WATCHDOG=1` over an unbound datagram socket is `std::os::unix::net::UnixDatagram`, with `std::os::linux::net::SocketAddrExt` for an abstract-namespace name.

## Dependency policy

`deny.toml` makes the policy checkable: `cargo deny check` runs in CI and fails the build on a violation.

- `[advisories]`: `yanked = "deny"` with an empty `ignore` list. An ignore entry records a judgement about the deployment that no later run rechecks.
- `[licenses]`: an allow-list (`Apache-2.0`, `MIT`, `MIT-0`, `Zlib`) trimmed to what the dependency graph contains, at `confidence-threshold = 0.9`. A dependency arriving under an unlisted licence fails the check and needs a one-line edit with a reason. `Zlib` was added for `foldhash`, reached through `yaml-rust2`, `hashlink` and `hashbrown`. Change this list in the same commit as the licence it records, so it cannot disagree with what ships.
- `[bans]`: `multiple-versions = "warn"`, since two versions of one crate are the first sign of a set outgrowing what a reader can audit; `wildcards = "deny"`; and `syn`, `quote`, `proc-macro2` and `serde_derive` denied by name, so turning on serde's `derive` feature fails the check. For a four-field struct, `#[derive(Deserialize)]` saves about fifteen lines of field extraction and costs the whole macro toolchain on the critical path of every package build. If a future dependency needs procedural macros, delete these entries and give the reason in the commit.
- `[sources]`: crates.io only; unknown registries and git dependencies are denied. A git dependency has no version and no audit trail, which does not suit software expected to run untouched for the length of an exhibition.

Revisit the crate policy if a dependency's tree grows past what can be audited, or if the device stops being a viable build host.

## Toolchain floor

`rust-version = "1.85"` is the rustc Debian trixie ships, which is the compiler the devices have.

A Raspberry Pi is also a development host, so the crate must build with that compiler; CI uses apt's rustc to enforce it. A dependency needing a newer compiler fails the CI build. Raise the floor only after confirming the devices can still build the package.

CI installs cargo-deb from its 2.x line, because cargo-deb 3.7 uses let-chains and needs rustc 1.88 or newer. The pin follows from the toolchain floor; if the pin ever moves, the toolchain decision is what changed.

## Version and build identity

Two identifiers name a build: the package version apt compares, and the commit compiled into the binary.

CI runs `cargo deb --deb-revision "<run-number>+g<short-sha>"`, giving package versions of the shape `0.1.0-<run>+g<sha>`. The run number leads because dpkg compares digit runs numerically, so ordering follows time. A commit-only revision orders by hash instead: `dpkg --compare-versions` sorts `0.1.0-1+gzz999999` above `0.1.0-1+g000aaaaa`, and apt then refuses a newer build as a downgrade. apt also skips an identical version without installing it, so a hand-built package of the same version needs `apt install --reinstall`.

`deploy/changelog` is the Debian changelog: a non-native package without one is a lintian error, and it is where a technician reads what shipped. It records `0.1.0-1`, the first packaged release.

`build.rs` writes the commit into the binary as `DEX_GIT_HASH`, using the standard library and no crates; dexd prints it to standard error as its first line. `--version` is not an option: `dexd --version 2>&1 | head -1` reads that line back, then prints the usage text and exits 2 — see [reference](../guides/reference.md).

`build.rs` takes the first of three sources:

1. `DEX_BUILD_ID`, an environment variable, trimmed to 12 characters;
2. `.dex-build-id`, a one-line file written by an external sync step and never committed;
3. `git rev-parse --short=12 HEAD`, with `+dirty` appended when `git status --porcelain` reports uncommitted changes.

With none of them the build reports `nogit`, which leaves every device carrying that package unidentifiable. CI therefore sets `DEX_BUILD_ID` from the commit it is building and asserts that the packaged binary prints those 12 hex characters.

CI sets the environment variable rather than writing `.dex-build-id`. A `rerun-if-changed` path that did not exist when the cached build ran counts as never changed, so a job that restores a `target/` cache and then writes the file gets a binary with the old identity. Cargo compares the value of a `rerun-if-env-changed` variable, which has no such hole.

`build.rs` also emits `rerun-if-changed=src` and `=Cargo.toml`. Emitting any `rerun-if-changed` replaces Cargo's default rebuild-on-source-change, so without those two entries an edit without a commit would keep the previous hash.

## Maintainer scripts

`postinst` and `postrm` run as root on every device under `/bin/sh`, which is dash on Debian, so they contain no bashisms; `checkbashisms` runs in CI.

`postinst` works inside a `case "$1" in configure)` block: it creates the `dex` system user, adds it to `video` and `render` where those groups exist, and creates `/opt/dex` at mode 0755. It then prints a notice, without failing the install, in three cases: `/opt/dex` contains no exhibit config; a config sits under `/etc/dex`; a systemd drop-in still overrides the display mode with `--mode`, a setting that moved into the [exhibit config](exhibit-config.md). `postrm` keeps the `dex` user and `/opt/dex`: the video is content the package never shipped, and the user may be named in something a technician wrote.

[The systemd unit](service-unit.md) covers what each step is for. The package installs, starts, plays, stops, removes and purges on a Raspberry Pi (measured; see [the measurement record](measurements.md)). CI's lifecycle test repeats the install, remove and purge steps in a clean trixie container.

## Lintian

Lintian runs as `lintian --tag-display-limit 0 --fail-on error,warning`, so a finding fails the build. The package reports three tags; `deploy/lintian-overrides` silences them, with the reason for each, and ships inside the package.

- `aliased-location`: cargo-deb 2.12 writes the unit to `lib/systemd/system`. On the merged-usr layout that trixie uses, that is `usr/lib/systemd/system`, the same directory through a symlink, and systemd resolves the unit there. Delete this override when the cargo-deb pin moves.
- `initial-upload-closes-no-bugs`: the tag wants the changelog to close an intent-to-package bug, a rule for packages uploaded into the Debian archive. The dexd package is built in CI and installed on the project's own devices.
- `embedded-library libyaml`, listed once per compiled binary: lintian tests for the string `did not find expected <stream-start>`, one of libyaml's error messages, which `yaml-rust2` reproduces because it is a Rust port of libyaml's scanner and parser. The arm64 binary embeds no libyaml (measured; see [the measurement record](measurements.md)). Re-check the override if the YAML dependency is ever swapped for one with a C backend.

## Licensing

The source, the packaging and the documentation are MIT-0; content (test cards, video masters, branding) is CC0-1.0; GPL appears only where it was inherited, through pi-gen into dexOS. The intent is to release the project's works to the public domain as far as is legally and practically possible, and both licences ask nothing of a reuser.

The shipped .deb is a GPL-3+ combined work: it links Debian's libmpv, which links GPL-3+ libsmbclient. MIT-0 is GPL-compatible, so this source imposes no condition on anyone; the binary's terms follow from how a distributor builds mpv, and an mpv built without libsmbclient yields an LGPL-2.1+ combination. `LICENSE` states the source's terms and the binary's, and ships verbatim as the package's copyright file, so the distinction travels with the package.

Per-file licensing is machine-readable through a single `REUSE.toml` following the REUSE specification, and `reuse --root . lint` runs in CI. One author under one licence makes a per-file header a restatement of the same fact in every file; per-file SPDX headers become the better choice the moment the crate mixes licences or takes third-party code. Repository-wide compliance waits on the GPL code inherited through pi-gen; annotating it would state a claim about someone else's licence rather than record one.

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| `/usr/local/bin` for the binaries | Installs and runs | Debian policy reserves `/usr/local` for the local administrator |
| The symbol-derived floor alone | `libmpv2 (>= 0.19.0)` | Installs on a device whose mpv lacks the behaviour dexd needs |
| A stock exhibit config as a dpkg-managed configuration file | dpkg preserves edits across upgrades | The config belongs beside the video, where the card shows it on any computer |
| A video inside the package | One file to install | Changing the video would mean rebuilding the software |
| serde with `derive` | About fifteen lines fewer | The syn, quote and proc-macro2 toolchain on every package build |
| `serde_yaml` | A YAML parser | Archived upstream |
| `serde_yaml_ng` | A YAML parser | Ten added crates |
| `saphyr` | A YAML parser | Twenty added crates and six procedural macros |
| JSON syntax only, no YAML parser | No added crate | The file a technician edits loses comments |
| An `sd-notify` crate or a libsystemd binding | About 150 lines fewer | A new shared-library link in `Depends`, and the ping policy stays here anyway |
| `.dex-build-id` as CI's build identity | One file to write | A path absent during the cached build never counts as changed |
| A commit-only package revision | A shorter version | Orders by hash, so apt can refuse a newer build |
| Hand-written unit enable and disable logic | Avoids the `aliased-location` tag | Reimplements cargo-deb's enable and disable handling, which runs on every install |
| Per-file SPDX headers | No `REUSE.toml` | The same fact restated in every file, above dense module docs |
| CC0-1.0 for code | One licence for everything | Fedora disallows CC0 for code, which forfeits a distribution channel |
| MIT, BSD-2-Clause | The same permissive intent | Require attribution |
| 0BSD, Unlicense | No attribution either | 0BSD is the same intent in different drafting; Unlicense is criticised as poorly drafted |
| GitHub Packages for distribution | One home for artifacts | It carries no Debian or apt repositories |
