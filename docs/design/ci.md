# Continuous integration

This page describes the workflow in `.github/workflows/dexd.yml`, which builds and checks the dexd Debian package for the Raspberry Pi players. It covers the build environment, what each job asserts and what a passing run establishes. It is for anyone changing the crate, the package or the workflow.

## Build environment

The package builds on an `ubuntu-24.04-arm` runner inside a `debian:trixie` container. The runner gives native arm64 with no emulation; the container gives Debian's libraries, the ones the players have.

The pairing matters because the package derives `Depends:` from the shared libraries the binary links, through `dpkg-shlibdeps` (see [packaging](packaging.md)). Linking against Ubuntu's libmpv and installing on Debian would produce the version mismatch that a derived `Depends:` exists to catch. Re-check the pairing when the players move to a new Debian release, by running on a player:

```sh
. /etc/os-release; echo $VERSION_CODENAME; dpkg -s libmpv2
```

The codename must be the container's Debian release, and the player's libmpv2 at least the version the build log records.

## Toolchain

`rustc` and `cargo` come from Debian's archive inside the container. Trixie ships Rust 1.85, and `rust-version = "1.85"` in `Cargo.toml` states the same floor. The `build` job enforces it: a dependency needing a newer compiler fails the build here, not at deploy time on a player.

cargo-deb is pinned to its 2.x line, because version 3.7 needs Rust 1.88. The install step tests for the binary itself rather than for `cargo-deb` on `PATH`:

```sh
test -x "$HOME/.cargo/bin/cargo-deb" || cargo install --locked cargo-deb --version "^2"
```

`cargo` comes from apt here, so nothing puts `~/.cargo/bin` on `PATH`. On a cache hit `command -v cargo-deb` reports the binary missing while `cargo install` exits 101 with the binary already in place. `cargo deb` runs either way, because cargo searches `$CARGO_HOME/bin` for its subcommands.

The `build` job installs build-essential, pkg-config, ca-certificates, git, rustc, cargo, rust-clippy, libmpv-dev, dpkg-dev, lintian and ffmpeg.

The job records `dpkg -s libmpv2`, `rustc --version` and `cargo --version` in the log. When a later package refuses to install on a player, compare those lines with the player's own output first.

`ffprobe -version` runs in the same step. The `dex-sidecar write` tests encode HEVC with ffmpeg and skip themselves when it is missing, and a skipped test still reports a pass.

Cargo caches `~/.cargo/bin`, `~/.cargo/registry`, `~/.cargo/git` and `packages/dexd/target`, keyed on a hash of `Cargo.lock` with the prefix `dexd-deb-` as restore key.

## Triggers and change detection

The workflow runs on manual dispatch, on pushes to `main`, `experiment/**` and `feature/**`, on tags matching `dexd-v*`, and on every pull request. CI runs on branch pushes because a check that runs only at release time cannot prevent the release. Manual dispatch appears in the Actions interface only for workflows on the default branch.

The workflow carries no path filters. A filtered workflow that does not run reports nothing, and a required check that never reports blocks every merge.

The `changes` job decides what is relevant with `git diff` against a base commit. It checks out with `fetch-depth: 0`, because the diff needs history.

`changes` publishes two flags:

- `crate`, set when a path under `packages/dexd/` or the workflow file changed. `build`, `lint`, `deny` and `lifecycle` run on it.
- `docs`, set when `docs/`, `AGENTS.md`, `CLAUDE.md`, the docs lint tool or the workflow file changed. `docs-lint` runs on it.

Three cases set both flags whatever the diff says: a manual dispatch, a tag build, and an unusable diff base. A release is built from the whole tree, a person pressing the button means run it, and an unknown diff must not skip a real change. The base is `github.event.pull_request.base.sha` for a pull request and `github.event.before` otherwise. An empty value, an all-zero hash, or a commit this checkout lacks — a new branch, a force-push, a first commit — is unusable.

Workflow permissions are `contents: read`.

The workflow groups concurrency per ref as `dexd-deb-${{ github.ref }}` with `cancel-in-progress`, so a push supersedes the previous run on the same branch. A tag build has its own ref, so the run that produces the release artifact survives an unrelated branch push.

## The required check

`gate`, displayed as **required**, is the single job to mark required in branch protection. It runs with `if: always()`, because a check that can be skipped can never be required, and it depends on `changes`, `build`, `lint`, `deny`, `lifecycle` and `docs-lint`. It decides in shell, which separates a legitimate skip from a failure and from a cancellation:

- any needed job whose result is `failure` or `cancelled` fails the check;
- a `skipped` job fails the check when its flag was true, because a job skipped by a failed dependency is not a legitimate skip;
- otherwise the step logs `gate: PASS`.

## Package build

The `build` job compiles, tests and packages:

```sh
cargo clippy --all-targets -- -D warnings
cargo test --release
```

The full suite runs here because the container has libmpv. A checkout on macOS cannot link the binary, so `cargo test --lib` is the most a developer machine runs; see [building and testing dexd](development.md).

### Forced-recovery test

One step runs a single test from `tests/cli.rs` by exact name:

```sh
cargo test --release --test cli -- --ignored --exact force_recovery_survives_against_real_mpv
```

The test drives a real mpv core with software HEVC decode and `vo=null`, forces an in-place recovery, and asserts the process survives the `END_FILE` (reason=stop) event that its own recovery produces. Trixie's software decoder supplies the one environment-dependent ingredient, so no display, DRM device or GPU is needed. The test carries `#[ignore]`, so a plain `cargo test` never starts a real-decode run on a player in the middle of a long-running test; this step is where it runs.

The step greps the output for `1 passed`. libtest exits 0 when a filter matches nothing, so a renamed or deleted test would leave the step passing having run nothing; requiring the string turns that into a failure.

Breaking the check confirms it (see [Vacuous checks](#vacuous-checks)). Disabling the increment that marks a recovery's own `END_FILE` as expected fails this step in about three seconds. The clippy step and the rest of the suite still pass (see the [measurement record](measurements.md)).

### Package assertions

`cargo deb` builds the .deb with a per-build revision:

```sh
short=$(printf '%s' "$GITHUB_SHA" | cut -c1-12)
cargo deb --deb-revision "${GITHUB_RUN_NUMBER}+g${short}"
```

The run number leads because dpkg compares runs of digits numerically, so version order follows time; a revision built from the commit alone does not (see [packaging](packaging.md)).

dpkg-shlibdeps derives `>= 0.19.0` from the linked symbols, while `Cargo.toml` states `libmpv2 (>= 0.40.0)` because the requirement is behaviour, not symbols (see [packaging](packaging.md)).

The job prints the package's fields and contents into the log, then asserts:

| Assertion | Failure it catches |
|---|---|
| `Depends` mentions libmpv | dpkg-shlibdeps stopped resolving libmpv, so the package installs on a player that has none |
| `Depends` matches `libmpv2 (>= 0.40)` or higher | the `libmpv2 (>= 0.40.0)` line was dropped from `Cargo.toml`, so the package installs against a too-old libmpv |
| the first stderr line does not contain `nogit` | the packaged binary cannot name the commit it was built from |
| the first stderr line contains `($short)` or `($short+dirty)`, `$short` being the 12-hex commit | the same, positively: an empty string also lacks `nogit` |
| `Version` contains `$GITHUB_RUN_NUMBER+g$short` | apt treats the version as already installed and leaves the older binary running |

The `Verify derived dependencies` step depends on three details:

- `dpkg-deb -x` extracts the whole tree. Piping `dpkg-deb --fsys-tarfile` into `tar` fails, because the tarfile's members carry no `./` prefix, so `tar -xO ./usr/bin/dexd` matches nothing and exits 2.
- dexd prints its version and build identity to stderr as its first line, whatever the arguments, so the capture uses `2>&1`. Without it the captured string is empty, the `case` matches nothing, and the check reports success having observed nothing.
- `cut` truncates the commit hash. Run steps in a `container:` job execute under `/bin/sh` (dash on trixie), where `${VAR:0:12}` is a "Bad substitution" error.

The workflow sets `DEX_BUILD_ID` to `github.sha`, and `build.rs` compiles it into the binary as its build identity. The workflow passes it in an environment variable rather than a build-id file. `rerun-if-changed` on a path absent when the cached build ran counts as unchanged. With a build-id file, a package built from a warm cache reports `(nogit)`.

`lintian --tag-display-limit 0 --fail-on error,warning` then checks the package, with accepted tags and their reasons in `deploy/lintian-overrides`. The .deb is uploaded as the artifact `dexd-deb`, with `if-no-files-found: error`.

## Static checks

`lint` covers the files that ship without being compiled. It is a separate job, so it still reports when the package build breaks. It installs shellcheck, devscripts (for checkbashisms), systemd (for systemd-analyze), git, ca-certificates and reuse.

- `systemd-analyze verify deploy/dexd.service`. systemd accepts a directive it does not recognise in a section without reporting it, so a typo or a misplaced key leaves the unit starting with the directive having no effect. The step fails on any output other than the two expected `Command ... is not executable` notices. Those binaries ship in the package, which this container does not install.
- `shellcheck deploy/dex-wait-hdmi deploy/maintainer-scripts/*` and `checkbashisms deploy/maintainer-scripts/*`. Maintainer scripts run as root on every player under `/bin/sh`, where a bashism fails the install.
- `reuse --root . lint`. Every file in the crate carries its copyright and licence machine-readably. The scope is the crate; the repository as a whole waits on the GPL-inherited packages.

## Dependency policy

`deny` runs `cargo deny check` against `deny.toml`: security advisories, a licence allow-list trimmed to what the dependency graph contains, and a ban on the procedural-macro toolchain. Without this job, nothing would enforce the decision to avoid derive macros.

The job runs on the bare runner with rustup, outside the container. The Debian-compiler rule binds what builds the shipped artifact; cargo-deny produces nothing that ships and needs a newer rustc than trixie has. cargo-deb has the same property but builds the package, so that one is pinned instead.

## Package lifecycle

`lifecycle` downloads the `dexd-deb` artifact and installs, removes and purges it against a real dpkg in a clean trixie container. Maintainer scripts that fail, files that outlive a purge and a service user never created appear only on a real install and removal; no static linter reaches this class of bug.

The job runs the install/remove/purge/autoremove cycle twice, the first time unobserved. Installing the package pulls in systemd, dbus and policykit. These are Debian-protected packages that never autoremove, and their own postinst scripts create state — a machine-id, the systemd catalog, enablement markers — that no dpkg file list mentions. A player already carries them in its base image, so the first cycle brings the container to that starting state and the diff compares like with like.

The asserted cycle checks each step:

| Step | Present | Absent |
|---|---|---|
| after install | `/usr/bin/dexd` and `/usr/bin/dex-wait-hdmi`, both executable; the `dex` user; `/opt/dex`; `/lib/systemd/system/dexd.service`; `/usr/share/man/man1/dexd.1.gz` | a video asset; an exhibit config; `/etc/dex` |
| after `apt-get remove` | the `dex` user; `/opt/dex` | `/usr/bin/dexd` |
| after `apt-get purge` | the `dex` user; `/opt/dex` | the unit file; `/etc/dex` |

Group membership follows postinst's own conditional check, with one exception. udev creates `render`, and this container has no udev, so the job cannot assert membership in that group. Debian's `base-passwd` defines `video` in every Debian environment, so the job asserts unconditionally that `video` exists and that `dex` belongs to it. That keeps one real assertion in force.

The postrm script leaves two things behind on purpose: `/opt/dex`, which contains the video, its sidecar and the exhibit config — none of which the package shipped — and the `dex` user, which anything written at the venue may name.

A third leftover is a bug, so the job also compares filesystem snapshots from before the install and after the purge. `snap()` lists `find / -xdev` sorted, pruning `/proc`, `/sys`, `/run`, `/tmp`, `/var/log`, `/var/lib/apt`, `/var/cache`, `/var/lib/dpkg` and `/github`. `grep -v` removes the `/opt/dex` lines from both before the comparison. On any difference the step prints `::error::purge left files behind beyond the two intended:` with the diff.

Snapshots go under `/tmp`, because a snapshot written to `/` shows up in the next one as a new file. `diff` reads them from temporary files, because run steps in a `container:` job execute under dash, which has no process substitution.

## Documentation lint

`docs-lint` runs on the plain runner with Node 20: first the lint tool's own tests, then the tool over the public documentation, the glossary and the writing rules.

```sh
node --test scripts/docs-lint.test.mjs
node scripts/docs-lint.mjs --coinages docs/lint-coinages.tsv docs AGENTS.md
```

## Vacuous checks

Confirm a check by breaking what it protects: disable the code path, confirm the step fails for the stated reason, restore the code, confirm it passes. A check that observes nothing looks the same as a check that passes.

A vacuous check fails in one of two ways:

| Shape | Examples |
|---|---|
| The check observes the wrong thing | a grep matching the echoed command instead of its output; an assertion on a variable that captured empty stderr |
| The check never runs | a path-filtered job; a tool with no build for the runner's architecture; an assertion on a group a minimal container cannot have |

In the Actions interface, neither a job that never ran nor a job that passed reports a failure.

A result from a developer's own machine is likewise a claim about that machine's toolchain. Clippy reports errors on Linux/aarch64 that macOS does not, because `c_char` is `u8` on one and `i8` on the other, and Debian's shellcheck reports findings other builds do not. The container's result decides. The code passes on every version of these linters, so none is pinned.

## Limits

A passing run establishes:

- dexd builds with the compiler and the libraries the players have;
- its test suite and the forced-recovery test pass;
- the package installs, removes and purges on trixie/arm64, leaving only the two intended leftovers;
- the artifact carries a distinct version and a traceable commit.

It does not establish that the picture comes back after a recovery. The forced-recovery test runs with `--no-defaults` and `vo=null`, so hardware decode, the drmprime-overlay interop and the plane swap are never exercised. A recovery that rebuilds that chain incorrectly leaves a black screen on a live process. Only a run on a Raspberry Pi with a display attached covers that — see [failure handling](failure-handling.md).

No job runs on Raspberry Pi hardware, so playback throughput, loop-point behaviour and thermal results come from the [measurement record](measurements.md). The lifecycle job tests no upgrade from a previous version, and its filesystem diff is narrower than piuparts's leftover heuristics.

## Alternatives

| Option | Outcome |
|---|---|
| Path filters on the workflow | Rejected: a check that never reports blocks every merge |
| A third-party change-detection action | Rejected: `git diff` is a few lines and one dependency fewer |
| rustup toolchain inside the container | Rejected: the target's libraries with another compiler |
| Native build on the runner's Ubuntu image | Rejected: links against Ubuntu's libmpv |
| Cross-build or qemu on an x86 runner | Rejected: arm64 runners are free for public repositories |
| piuparts for the lifecycle test | Rejected: no installation candidate for the runner's architecture, and the container is already a throwaway environment |
| Pinning linter versions | Rejected: the code is version-independent instead |
