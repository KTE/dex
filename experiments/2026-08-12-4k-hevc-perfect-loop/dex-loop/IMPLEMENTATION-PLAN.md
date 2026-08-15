# dex-loop — implementation plan (code-level hardening)

Status: ready to execute, 2026-08-15. Derived from [PLAN.md](PLAN.md), which stays as the
design record. This document is the executable version of PLAN.md's **T0, T1, T2, T3, F3,
F4, F7** — nine sequential tasks, each leaving the crate compiling and its tests passing.
**F1, F2, F5, F6 are deliberately NOT implemented here** (see §4).

Facts below marked *(verified 2026-08-15)* were checked against the live bench, not
transcribed from memory: libmpv strings via `ctypes` on the Pi, SHA-256 vectors via
`shasum -a 256`, `cargo check`'s no-link behaviour on the Mac.

---

## 1. What changed from PLAN.md, and why (critique)

Executing PLAN.md as written would have produced three tests that fail against the real
world and one that silently tests nothing. Each change below is a correction, not a
preference:

1. **T1's invariant "`pos` always `< len` after any call" is FALSE for the shipped code.**
   `read_fn` wraps lazily: after returning the tail it leaves `pos == len` and wraps at the
   *start* of the next call. A faithful extraction would fail the planned test. Fixed by
   making `next_chunk` wrap **eagerly** (`next_pos` is always `< len`); the produced byte
   stream is identical, and the invariant becomes true instead of the test becoming weaker.

2. **T0's proposed signature `fn next_chunk(len, pos, want) -> (usize, usize)` cannot
   express its own contract.** It loses the copy *start* offset (the wrapped position), so
   the unsafe shell would re-derive the wrap — leaving the load-bearing arithmetic exactly
   where it was untestable. And "never returns 0" plus "want == 0 handled explicitly" need a
   distinct error outcome. Changed to `fn next_chunk(len, pos, want) -> Option<Chunk>` with
   `Chunk { start, n, next_pos }`; `None` = "caller must return an mpv error, never 0".

3. **T2's assertion `error_string(MPV_ERROR_UNSUPPORTED).contains("unsupported")` fails
   against the real library.** *(verified 2026-08-15, mpv 0.40.0 on the Pi, via ctypes)*:
   `mpv_error_string(-18)` is `"not supported"` — with a space. The test now asserts
   `contains("supported")` (robust across that wording) plus that `-18` differs from `-1`
   (`EVENT_QUEUE_FULL`, the sign-compatible wrong constant this code once used). Event
   names were verified exact: `0 none, 1 shutdown, 2 log-message, 6 start-file, 7 end-file`.

4. **T3's "garbage bytes → exits non-zero" row quietly changes meaning once F4 lands** —
   after the NAL gate, garbage never reaches mpv at all, so the row would stop testing the
   event loop (where bug #1 lived) and start testing the gate. Split into two tests:
   garbage → gate refusal (exit 2, bounded), and a **dedicated never-hangs regression** that
   forces a *post-gate* playback failure with `--opt vid=no` (track deselected → mpv ends
   with `NOTHING_TO_PLAY` → `END_FILE`). That is deterministic, display-free, and exercises
   the exact idle-hang path of bug #1.

5. **T3's "truncated valid stream → refused once F4 lands" credits the wrong fix.** A
   truncated copy has *intact leading NALs* — F4 passes it. Truncation is caught by the F3
   hash, as PLAN.md's own F4 text states. The test binds a full-asset hash to a truncated
   payload and asserts the *hash* refusal.

6. **F3's "takes fps/mode from it": binding `--mode` to the asset is wrong and is dropped.**
   Mode is venue/display configuration, not asset metadata — the same 4K asset legitimately
   plays scaled on a 1080p bench monitor, and a sidecar-forced mode would fight the actual
   connector. The sidecar binds **fps + sha256** (the two undetectable-if-wrong facts);
   `--mode` stays a CLI/deploy concern.

7. **F3's fps must be a JSON *string*, not a number.** `29.97` as a float invites drift and
   `30000/1001` cannot be a JSON number at all. The sidecar carries `"fps": "30"` (or
   `"30000/1001"`), validated by grammar and passed verbatim to `container-fps-override`.

8. **F3's "`--fps` survives as a bench-only override" needs an explicit flag, not a
   fallback.** If `--fps` alone worked whenever the sidecar was missing, the deploy path
   could silently run unbound — the exact operator-error principle 5 forbids. The bench
   escape hatch is a deliberate two-flag act: `--bench-no-sidecar --fps <F>`. With a sidecar
   present, `--fps` is allowed only when it equals the sidecar value exactly (a stale
   wrapper script must fail loudly, not win silently). The systemd unit drops `--fps`.

9. **F4's "IRAP slice (16–23)" is too loose for the premise it protects.** The gaplessness
   argument everywhere in this crate is "IDR at frame 0". CRA (21) admits RASL leading
   pictures whose wrap-join correctness is content-dependent; BLA (16–18) never comes from a
   sane ingest; 22/23 are reserved. The gate requires the first VCL NAL to be **IDR (19 or
   20)** and names CRA specifically in its refusal. Relax knowingly if an asset ever
   justifies it.

10. **sha256 under the zero-dependency rule:** the hash runs at *runtime* (startup gate), so
    it cannot be a dev-dependency; adding `sha2` would break the crate's deliberate
    zero-dependency, builds-offline-on-the-Pi property for ~90 lines of stable, vectorable
    code. Hand-rolled, one-shot only, pinned by the NIST vectors (incl. the 1M-'a' vector)
    plus `shasum`-verified padding-boundary vectors at 55/56/63/64/65/112 bytes.

11. **Sidecar JSON is parsed by a hand-rolled STRICT SUBSET parser, fail-closed.** One flat
    object, string and unsigned-integer values, six escapes, nothing else. Anything outside
    the subset is a parse error and a parse error refuses startup — which is the F3 semantics
    anyway (an unparseable sidecar and a missing one are the same operational fact). Unknown
    *keys* are ignored so ingest can add metadata without breaking deployed players.

12. **The 32-bit truncation test (T1) cannot genuinely execute its interesting branch on a
    64-bit host** — `usize::try_from(u64)` never fails there, and the bench Pi runs aarch64.
    Kept, honestly labelled: it pins the contract against someone reintroducing `as usize`;
    a 32-bit target would then catch it.

13. **An exit-code contract was missing** and T3 needs one to assert against:
    **2 = refused before playback** (bad invocation / asset / sidecar; operator-fixable,
    restarting cannot help), **1 = playback/runtime failure** (supervisor restarts). Printed
    in `usage()`.

14. **F7's heartbeat would never fire when healthy** under the current event loop:
    `mpv_wait_event(ctx, -1.0)` blocks forever, and a healthy steady state delivers *no
    events*. The loop wakes every 30 s (`timeout = 30.0`). Also added: **heartbeat #0
    immediately after loadfile** — it proves the whole mechanism (property reads,
    temperature, formatting) on every boot, gives the journal a start anchor, and makes the
    mechanism testable without waiting 10 minutes.

15. **Known limitation, named rather than papered over:** the endless stream removes EOF,
    and EOF is where mpv detects many failures. A stream that *probes* as HEVC but never
    yields a decodable frame can leave mpv "buffering" indefinitely — in-process, that is
    only detectable by a stall watchdog, which is **F1 (out of scope here)**. The gates
    reduce the reachable set (garbage cannot probe, unbound assets are refused, hashes must
    match), but this plan does not claim to close that hole. It strengthens the case for F1.

16. **T3's "no display present" row cannot be automated now** — it needs the display, and a
    soak owns it. Recorded as a manual bench procedure in §4. Every automated test in this
    plan is display-free by construction (see the safety rule in §2).

---

## 2. Ground rules for every task

**Where things are.**

| What | Where |
|---|---|
| Crate (Mac, canonical) | `/Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop` |
| Git repo root / branch | `/Users/mfa/CODE/dex` — branch `experiment/4k-hevc-perfect-loop`, remote `origin` (github.com/KTE/dex) |
| Pi (build+test host) | `dexpi@dexpi4.local`, key `~/.ssh/id_ed25519`, crate mirror at `~/bench/dex-loop` |
| Toolchains *(verified)* | Mac: cargo 1.94.1, **no libmpv**. Pi: cargo 1.85.0, aarch64, libmpv-dev + mpv 0.40.0, ffmpeg 7.1.5 with libx265 |

**Verification recipe** (run after every implementation step):

```bash
# Mac — type-checks EVERYTHING including the libmpv bin and integration tests,
# without linking (verified: works with no libmpv installed):
cd /Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop
cargo check --all-targets

# Mac — run the pure-logic tests (lib only; never links libmpv):
cargo test --lib

# Pi — full suite including FFI + CLI integration tests (tasks 2, 4, 6, 7, 8):
rsync -a --delete --exclude target -e "ssh -i ~/.ssh/id_ed25519" \
  /Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/ \
  dexpi@dexpi4.local:~/bench/dex-loop/
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local \
  'cd ~/bench/dex-loop && nice -n 19 cargo test 2>&1 | tail -40'
```

A plain `cargo test` on the Mac fails at link time (no libmpv) — that is expected; use
`cargo test --lib` there. `nice -n 19` on the Pi keeps test builds from stealing CPU from a
running soak.

**Safety rule — the display belongs to the soak.** Never run the player against the
display, and never let a test do it either: **every `tests/cli.rs` invocation that can
reach `mpv_create` MUST carry `--no-defaults --opt vo=null --opt vid=no --opt aid=no`**
(the null VO never touches DRM; deselected tracks make mpv end deterministically). Tests
that exit before mpv (missing file, empty file, usage, gate refusals) are safe by
construction — but during a task's RED phase a not-yet-implemented gate means the
invocation *will* reach mpv, so the headless options are mandatory even on gate tests.

**TDD, per task.** Write the failing test → run it and watch it fail (a stub returning a
dummy value beats a compile error: the red is an assertion, not a typo) → minimal
implementation → run it green → commit. For regression tests of already-fixed bugs (tasks
2 and 4), prove the test bites by temporarily re-introducing the bug, watching the test
fail, and reverting.

**Commit convention** (matches this repo's log: `E:` code, `D:` docs, scope
`4k-loop-probe`):

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop
git commit -m "E: 4k-loop-probe — <what changed>"
git push origin experiment/4k-hevc-perfect-loop
```

Stage explicit paths only — the repo has unrelated submodule modifications; never
`git add -A`.

**Crate layout after all tasks:**

```
dex-loop/
  Cargo.toml               unchanged (zero dependencies, panic=abort both profiles)
  build.rs                 NEW (task 8): embeds git hash, std-only, degrades to "nogit"
  src/lib.rs               NEW: pure core, #![forbid(unsafe_code)]
  src/chunk.rs             NEW (task 1): next_chunk + clamp_want
  src/ffi_consts.rs        NEW (task 2): event ids + MPV_ERROR_UNSUPPORTED
  src/sha256.rs            NEW (task 3): one-shot SHA-256
  src/sidecar.rs           NEW (task 5): JSON-subset parser, Sidecar, resolve_fps, verify_payload
  src/nal.rs               NEW (task 7): validate_leading_nals
  src/heartbeat.rs         NEW (task 8): format_heartbeat
  src/main.rs              MODIFIED: thin unsafe shell + gates + event loop
  tests/ffi_constants.rs   NEW (task 2): live-library constant verification (Pi only)
  tests/cli.rs             NEW (task 4): failure-path integration tests (Pi only)
```

Note `cargo test` ignores `panic = "abort"` for test builds (documented Cargo behaviour),
so the profiles need no change. `src/lib.rs` + `src/main.rs` auto-discover as lib
`dex_loop` + bin `dex-loop`; `Cargo.toml` needs no target sections.

**Shared interface inventory** — every cross-task type and signature, fixed here so tasks
stay consistent:

```rust
// src/chunk.rs
pub struct Chunk { pub start: usize, pub n: usize, pub next_pos: usize }
pub fn next_chunk(len: usize, pos: usize, want: usize) -> Option<Chunk>;
pub fn clamp_want(nbytes: u64) -> usize;

// src/ffi_consts.rs  (all c_int = i32)
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;

// src/sha256.rs
pub fn sha256(data: &[u8]) -> [u8; 32];
pub fn sha256_hex(data: &[u8]) -> String;   // 64 lowercase hex chars

// src/sidecar.rs
pub enum Value { Str(String), Num(u64) }
pub fn parse_flat_json(text: &str) -> Result<Vec<(String, Value)>, String>;
pub struct Sidecar { pub fps: String, pub sha256: String,
                     pub width: Option<u64>, pub height: Option<u64> }
impl Sidecar { pub fn from_json(text: &str) -> Result<Sidecar, String>; }
pub fn is_valid_fps(s: &str) -> bool;
pub enum FpsSource { Sidecar, BenchOverride }
pub fn resolve_fps(sidecar_fps: Option<&str>, cli_fps: Option<&str>,
                   bench_no_sidecar: bool) -> Result<(String, FpsSource), String>;
pub fn verify_payload(payload: &[u8], sidecar: &Sidecar) -> Result<(), String>;

// src/nal.rs
pub const NAL_VPS: u8 = 32;  pub const NAL_SPS: u8 = 33;  pub const NAL_PPS: u8 = 34;
pub const NAL_IDR_W_RADL: u8 = 19;  pub const NAL_IDR_N_LP: u8 = 20;
pub fn validate_leading_nals(data: &[u8]) -> Result<(), String>;

// src/heartbeat.rs
pub fn format_heartbeat(wraps: u64, uptime_secs: u64, temp_millicelsius: Option<i64>,
                        frame_drops: Option<&str>, vo_delayed: Option<&str>) -> String;
```

**Exit-code contract:** `2` = refused before playback (usage, unreadable/empty asset,
sidecar missing/invalid/mismatched, NAL gate) — operator-fixable, restart cannot help.
`1` = playback/runtime failure (mpv init, END_FILE) — the supervisor restarts. The
sidecar path is always `<asset>.json` (`loop.265` → `loop.265.json`).

---

## 3. Tasks

Strictly sequential — they edit the same files.

---

### Task 1 — Extract `next_chunk` into a pure, fully-tested lib core (T0 + T1)

**Files.** Create `src/lib.rs`, `src/chunk.rs`. Modify `src/main.rs`. Tests live inside
`src/chunk.rs` (`#[cfg(test)]`), run on the Mac with `cargo test --lib`.

**Interfaces.**
Produces: `dex_loop::chunk::{Chunk, next_chunk, clamp_want}` (signatures in §2).
Consumes: nothing (pure). `src/main.rs::read_fn` becomes the only consumer.

**Step 1 (2 min).** Create `src/lib.rs`:

```rust
//! dex-loop's pure core: every piece of logic that can exist without libmpv.
//!
//! The binary (src/main.rs) is deliberately a thin unsafe shell over this
//! library: FFI structs, callbacks, and the event loop. Everything decidable —
//! wrap arithmetic, hashing, sidecar binding, NAL validation, log formatting —
//! lives here, testable on any machine with `cargo test --lib`, no libmpv and
//! no display required. That split is T0 of the hardening plan: every serious
//! bug so far lived where testing could not reach.

#![forbid(unsafe_code)]

pub mod chunk;
```

**Step 2 (4 min).** Create `src/chunk.rs` with STUB bodies and the full test module —
the stubs make the red phase an assertion failure, not a compile error:

```rust
//! T0 — the wrap arithmetic of the endless stream, extracted pure so it is
//! testable without libmpv. `read_fn` in main.rs is a thin unsafe shell over
//! `next_chunk`; the properties asserted here (never a zero-byte answer, the
//! concatenation property) ARE the program.

/// One read request's answer: copy `n` bytes starting at payload offset
/// `start`; the reader's position afterwards is `next_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// Offset into the payload to copy from. Always < payload length.
    pub start: usize,
    /// Bytes to copy. Never 0.
    pub n: usize,
    /// Reader position after the copy. Always < payload length: the wrap
    /// happens eagerly here, never lazily on the next call, so the invariant
    /// holds between calls.
    pub next_pos: usize,
}

/// Decide the next chunk of the endless loop.
///
/// `len` is the payload length, `pos` the reader position (any value is
/// tolerated; positions >= len wrap to 0 first), `want` the requested byte
/// count.
///
/// Returns `None` exactly when no bytes can be produced without lying: a
/// zero-length request (`want == 0`), or the impossible-after-startup empty
/// payload (`len == 0`). The caller MUST turn `None` into an mpv ERROR
/// return — never 0. To mpv, 0 means final EOF (stream_cb.h), the one event
/// this program exists to prevent, and "0 bytes requested" must never share a
/// return value with "the stream has ended".
///
/// A short read is legal (stream_cb.h), so the wrap is never stitched across
/// one call: the tail is returned now, the head on the next call.
pub fn next_chunk(len: usize, pos: usize, want: usize) -> Option<Chunk> {
    let _ = (len, pos, want);
    None // STUB — replaced in step 4
}

/// Clamp mpv's u64 request size to usize without ever turning a nonzero
/// request into 0. On a 32-bit target `as usize` truncates: an `nbytes` that
/// is an exact multiple of 2^32 would become a 0-byte request and therefore a
/// spurious final EOF. Saturating can only shrink the request, and short
/// reads are always legal.
pub fn clamp_want(nbytes: u64) -> usize {
    let _ = nbytes;
    0 // STUB — replaced in step 4
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1: never returns n == 0 for any want >= 1 (a 0 return = final EOF to
    // mpv, the exact event the design exists to prevent).
    #[test]
    fn never_zero_bytes_for_nonzero_want() {
        for len in [1usize, 2, 3, 7, 64, 1000] {
            for pos in [0usize, 1, len / 2, len.saturating_sub(1), len, len + 5] {
                for want in [1usize, 2, len, len + 1, 10 * len, usize::MAX] {
                    let c = next_chunk(len, pos, want)
                        .expect("want >= 1 on a non-empty payload must produce a chunk");
                    assert!(c.n >= 1, "n == 0 for len={len} pos={pos} want={want}");
                }
            }
        }
    }

    // T1: want == 0 (and the impossible len == 0) are explicit, distinct
    // outcomes — never conflated with a zero-byte "success" that mpv would
    // read as EOF.
    #[test]
    fn zero_want_and_empty_payload_are_none_not_zero_chunks() {
        assert_eq!(next_chunk(10, 0, 0), None);
        assert_eq!(next_chunk(10, 9, 0), None);
        assert_eq!(next_chunk(1, 0, 0), None);
        assert_eq!(next_chunk(10, 10, 0), None); // even at the wrap point
        assert_eq!(next_chunk(0, 0, 4096), None); // empty payload: error, not EOF
    }

    // T1: wrap at the exact payload boundary.
    #[test]
    fn wraps_at_exact_boundary() {
        // pos at end-of-payload (legacy lazy-caller state): wraps to 0 first.
        let c = next_chunk(10, 10, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 4, 4));
        // tail shorter than want: short read of the tail, next_pos wraps to 0.
        let c = next_chunk(10, 8, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (8, 2, 0));
        // read ending exactly at len: next_pos is 0, not len (eager wrap).
        let c = next_chunk(10, 6, 4).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (6, 4, 0));
    }

    // T1: payload smaller than the request.
    #[test]
    fn payload_smaller_than_want() {
        // whole payload in one request: short read of everything, wrap to 0.
        let c = next_chunk(3, 0, 4096).unwrap();
        assert_eq!((c.start, c.n, c.next_pos), (0, 3, 0));
        // single-byte payload: every read returns that byte, forever.
        for _ in 0..5 {
            let c = next_chunk(1, 0, 4096).unwrap();
            assert_eq!((c.start, c.n, c.next_pos), (0, 1, 0));
        }
    }

    // T1: the position invariant read_fn's SAFETY comment relies on.
    #[test]
    fn next_pos_always_less_than_len() {
        for len in [1usize, 2, 3, 5, 64, 4096] {
            for pos in 0..=len + 2 {
                for want in [1usize, 2, 3, len, len + 1, 3 * len] {
                    let c = next_chunk(len, pos, want).unwrap();
                    assert!(c.next_pos < len, "next_pos={} len={len}", c.next_pos);
                    assert!(c.start < len);
                    assert!(c.start + c.n <= len);
                }
            }
        }
    }

    // T1: THE property — driving next_chunk repeatedly reproduces the payload
    // repeated endlessly, byte for byte, for arbitrary request sizes. This is
    // the only property that matters: the stream really is the loop.
    #[test]
    fn concatenation_reproduces_the_endless_loop() {
        let payload: Vec<u8> = (0u8..=250).cycle().take(997).collect(); // prime length
        // deterministic pseudo-random request sizes (LCG, no dependencies)
        let mut rng: u64 = 0x853c49e6748fea9b;
        let mut random_sizes = Vec::new();
        for _ in 0..2000 {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            random_sizes.push(((rng >> 33) % 300 + 1) as usize); // 1..=300
        }
        let schedules: Vec<Vec<usize>> = vec![
            vec![1; 3000],  // one byte at a time
            vec![997; 8],   // exactly the payload length
            vec![996; 8],   // one short of the payload
            vec![998; 8],   // one past the payload
            vec![4096; 8],  // far larger than the payload
            random_sizes,   // pseudo-random schedule
        ];
        for schedule in schedules {
            let mut pos = 0usize;
            let mut out = Vec::new();
            for want in &schedule {
                let c = next_chunk(payload.len(), pos, *want).unwrap();
                out.extend_from_slice(&payload[c.start..c.start + c.n]);
                pos = c.next_pos;
            }
            let expected: Vec<u8> =
                payload.iter().copied().cycle().take(out.len()).collect();
            assert_eq!(out, expected, "stream diverged from the endless loop");
        }
    }

    // T1: the u64 -> usize clamp saturates rather than truncating to 0.
    // Honest limitation: on a 64-bit host try_from always succeeds, so the
    // truncation branch only genuinely executes on a 32-bit target (the Pi 4
    // runs aarch64). This pins the contract against someone reintroducing
    // `as usize`; a 32-bit CI target would then catch it.
    #[test]
    fn clamp_want_never_zero_for_nonzero_input() {
        assert_eq!(clamp_want(0), 0);
        assert_eq!(clamp_want(1), 1);
        assert_eq!(clamp_want(4096), 4096);
        assert!(clamp_want(1u64 << 32) != 0, "2^32 must not clamp to 0");
        assert!(clamp_want(u64::MAX) != 0);
        assert_eq!(clamp_want(u64::MAX), usize::MAX);
    }
}
```

**Step 3 (1 min).** Red:

```bash
cd /Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop
cargo test --lib
```

Expect failures in all tests except `zero_want_and_empty_payload...` (the stub's `None`
accidentally satisfies it) — the point is seeing the suite execute and fail on
assertions, proving the tests run.

**Step 4 (2 min).** Replace the two stub bodies in `src/chunk.rs` with the real
implementations (doc comments unchanged):

```rust
pub fn next_chunk(len: usize, pos: usize, want: usize) -> Option<Chunk> {
    if len == 0 || want == 0 {
        return None;
    }
    let start = if pos >= len { 0 } else { pos };
    let avail = len - start; // >= 1, because start < len
    let n = want.min(avail); // >= 1, because want >= 1 and avail >= 1
    let end = start + n; // <= len
    let next_pos = if end == len { 0 } else { end };
    Some(Chunk { start, n, next_pos })
}
```

```rust
pub fn clamp_want(nbytes: u64) -> usize {
    usize::try_from(nbytes).unwrap_or(usize::MAX)
}
```

**Step 5 (1 min).** Green: `cargo test --lib` — all 7 tests pass.

**Step 6 (4 min).** Rewire `src/main.rs`. Three edits:

(a) Immediately after the crate-level `//!` doc block (after the line
`//! * The real frame rate, ...` paragraph ends, before `use std::env;`), insert:

```rust
#![deny(unsafe_op_in_unsafe_fn)]
```

(b) After the existing `use std::process::ExitCode;` line, add:

```rust
use dex_loop::chunk::{clamp_want, next_chunk};
```

(c) Replace the whole `read_fn` — from the line
`/// Copy the next bytes out of the loop, wrapping at the end.` down to the closing brace
after `n as i64` — with:

```rust
/// The stream read callback: a thin unsafe shell over
/// [`dex_loop::chunk::next_chunk`], which owns (and tests) every rule that
/// matters — never return 0 (to mpv, 0 is final EOF, the one event this
/// program exists to prevent), wrap eagerly, report a zero-length request as
/// an error rather than 0, saturate the u64 request size. This function only
/// performs the memcpy the pure core cannot.
extern "C" fn read_fn(cookie: *mut c_void, buf: *mut c_char, nbytes: u64) -> i64 {
    // SAFETY: `cookie` is the Box<LoopStream> leaked in `open_fn`, and mpv
    // guarantees it is passed back unmodified for the life of the stream.
    let s = unsafe { &mut *(cookie as *mut LoopStream) };

    let Some(c) = next_chunk(s.data.len(), s.pos, clamp_want(nbytes)) else {
        // Zero-length request (or an impossible empty payload). Report an
        // error, never 0.
        return MPV_ERROR_UNSUPPORTED;
    };

    // SAFETY: mpv guarantees `buf` is writable for `nbytes` bytes; next_chunk
    // guarantees c.n >= 1, c.n <= nbytes (the request is clamped, never
    // grown) and c.start + c.n <= data.len(), and the ranges cannot overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(s.data.as_ptr().add(c.start), buf as *mut u8, c.n);
    }
    s.pos = c.next_pos;
    c.n as i64
}
```

**Step 7 (2 min).** Verify: `cargo check --all-targets && cargo test --lib` on the Mac.
Then link-verify on the Pi (also runs the lib tests there):

```bash
rsync -a --delete --exclude target -e "ssh -i ~/.ssh/id_ed25519" \
  /Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/ \
  dexpi@dexpi4.local:~/bench/dex-loop/
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local \
  'cd ~/bench/dex-loop && nice -n 19 cargo test 2>&1 | tail -20'
```

**Step 8 (1 min).** Commit (Cargo.lock is currently untracked — include it):

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/chunk.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/Cargo.lock
git commit -m "E: 4k-loop-probe — T0/T1: extract next_chunk pure core, test the loop invariants"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 2 — FFI constants verified against the live libmpv (T2)

**Files.** Create `src/ffi_consts.rs`, `tests/ffi_constants.rs`. Modify `src/lib.rs`,
`src/main.rs`. The new test target links libmpv → runs on the Pi only.

**Interfaces.**
Produces: `dex_loop::ffi_consts::{MPV_EVENT_NONE, MPV_EVENT_SHUTDOWN,
MPV_EVENT_LOG_MESSAGE, MPV_EVENT_START_FILE, MPV_EVENT_END_FILE,
MPV_ERROR_UNSUPPORTED}` — all `std::ffi::c_int`.
Consumes (test only, its own extern block): `mpv_event_name(c_int) -> *const c_char`,
`mpv_error_string(c_int) -> *const c_char` — static table lookups in libmpv; no mpv
instance is created and the display is never touched.

**Step 1 (3 min).** Create `src/ffi_consts.rs`:

```rust
//! libmpv ABI constants, transcribed from mpv/client.h (mpv v0.40.0) and —
//! more importantly — verified against the LIVE library at test time by
//! tests/ffi_constants.rs via mpv_event_name()/mpv_error_string().
//!
//! Why the paranoia: an earlier revision transcribed MPV_EVENT_LOG_MESSAGE as
//! 6. It is 2; 6 is MPV_EVENT_START_FILE. The event handler then cast a
//! start-file payload to a log-message struct and dereferenced garbage
//! pointers — a segfault on the first frame. A transcription can silently
//! rot; the live library cannot. These are NOT sequential-by-category.

use std::ffi::c_int;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;

/// The documented "not supported" sentinel for stream callbacks. `-1` is
/// MPV_ERROR_EVENT_QUEUE_FULL, which happens to work only because mpv 0.40
/// tests the sign rather than the value.
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;
```

**Step 2 (1 min).** In `src/lib.rs`, after `pub mod chunk;` add:

```rust
pub mod ffi_consts;
```

**Step 3 (4 min).** In `src/main.rs`:

(a) After the `use dex_loop::chunk::...;` line, add:

```rust
use dex_loop::ffi_consts::{
    MPV_ERROR_UNSUPPORTED, MPV_EVENT_END_FILE, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
};
```

(b) Delete the local constant block — everything from the comment line
`// Verified against mpv v0.40.0 client.h. These are NOT sequential-by-category:` through
the line `const MPV_ERROR_UNSUPPORTED: i64 = -18;` (including the `/// \`MPV_ERROR_UNSUPPORTED\`...`
doc comment above it).

(c) The constant is now `c_int` (i32), and the three callbacks return `i64` — fix the
three return sites:

In `read_fn`: `return MPV_ERROR_UNSUPPORTED;` → `return i64::from(MPV_ERROR_UNSUPPORTED);`

In `seek_fn`: the body `MPV_ERROR_UNSUPPORTED` → `i64::from(MPV_ERROR_UNSUPPORTED)`

In `size_fn`: the body `MPV_ERROR_UNSUPPORTED` → `i64::from(MPV_ERROR_UNSUPPORTED)`

**Step 4 (1 min).** Mac: `cargo check --all-targets && cargo test --lib` — green.

**Step 5 (5 min).** Create `tests/ffi_constants.rs`:

```rust
//! T2 — verify the hand-transcribed FFI constants against the LIVE libmpv.
//!
//! This target links libmpv, so it builds and runs ONLY where libmpv-dev is
//! installed (the Pi: `cargo test`). On the Mac use `cargo test --lib`. It
//! creates no mpv instance and never touches the display: mpv_event_name()
//! and mpv_error_string() are static table lookups.
//!
//! Why this exists: MPV_EVENT_LOG_MESSAGE was once transcribed as 6 (it is 2;
//! 6 is START_FILE). The handler cast a start-file payload to a log-message
//! struct and segfaulted on the first frame. mpv exposes the id->name mapping
//! at runtime, so assert the transcription instead of trusting it — this also
//! catches a future mpv renumbering, which no header copy can.

use std::ffi::{c_char, c_int, CStr};

use dex_loop::ffi_consts::{
    MPV_ERROR_UNSUPPORTED, MPV_EVENT_END_FILE, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
};

#[link(name = "mpv")]
extern "C" {
    fn mpv_event_name(event: c_int) -> *const c_char;
    fn mpv_error_string(error: c_int) -> *const c_char;
}

/// mpv_event_name returns NULL for ids it does not know.
fn event_name(id: c_int) -> Option<String> {
    let p = unsafe { mpv_event_name(id) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn error_string(code: c_int) -> String {
    // mpv_error_string is documented to always return a valid string.
    unsafe { CStr::from_ptr(mpv_error_string(code)) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn event_ids_match_the_live_library() {
    // Verified against mpv v0.40.0 on the bench Pi (2026-08-15, via ctypes):
    //   0 none, 1 shutdown, 2 log-message, 6 start-file, 7 end-file
    assert_eq!(event_name(MPV_EVENT_NONE).as_deref(), Some("none"));
    assert_eq!(event_name(MPV_EVENT_SHUTDOWN).as_deref(), Some("shutdown"));
    assert_eq!(event_name(MPV_EVENT_LOG_MESSAGE).as_deref(), Some("log-message"));
    assert_eq!(event_name(MPV_EVENT_START_FILE).as_deref(), Some("start-file"));
    assert_eq!(event_name(MPV_EVENT_END_FILE).as_deref(), Some("end-file"));
}

#[test]
fn the_exact_bug_that_shipped_cannot_recur() {
    // The historical defect: LOG_MESSAGE transcribed as 6, which is
    // START_FILE. Pin the two apart, in both directions.
    assert_ne!(MPV_EVENT_LOG_MESSAGE, MPV_EVENT_START_FILE);
    assert_ne!(
        event_name(MPV_EVENT_LOG_MESSAGE),
        event_name(MPV_EVENT_START_FILE)
    );
    assert_ne!(
        event_name(6).as_deref(),
        Some("log-message"),
        "6 is start-file, not log-message"
    );
}

#[test]
fn error_unsupported_matches_the_live_library() {
    // The live mpv 0.40 string for -18 is "not supported" — NOT "unsupported";
    // PLAN.md's original assertion would have failed here. Match loosely
    // enough to survive that wording family, and pin that -18 is not the
    // sign-compatible-but-wrong EVENT_QUEUE_FULL (-1) this code once used.
    let s = error_string(MPV_ERROR_UNSUPPORTED);
    assert!(s.contains("supported"), "mpv_error_string(-18) = {s:?}");
    assert_ne!(error_string(MPV_ERROR_UNSUPPORTED), error_string(-1));
}
```

**Step 6 (3 min).** Run on the Pi (rsync + `nice -n 19 cargo test` per §2) — all green,
including the three new tests.

**Step 7 (4 min).** Prove the test bites (regression tests get their red phase by
re-introducing the bug): in `src/ffi_consts.rs` temporarily change
`pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;` to `= 6;`, rsync, run on the Pi — expect
`event_ids_match_the_live_library` and `the_exact_bug_that_shipped_cannot_recur` to FAIL.
Revert to `= 2;`, rsync, green again.

**Step 8 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/ffi_consts.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/tests/ffi_constants.rs
git commit -m "E: 4k-loop-probe — T2: FFI constants verified against the live libmpv"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 3 — Zero-dependency SHA-256 (F3 prerequisite)

**Files.** Create `src/sha256.rs`. Modify `src/lib.rs`. Tests inside the module, Mac.

**Interfaces.** Produces `dex_loop::sha256::{sha256, sha256_hex}` (§2). Consumes nothing.
Runtime use (task 6) is why this cannot be a dev-dependency; hand-rolled per §1.10.

**Step 1 (5 min).** Create `src/sha256.rs` with a stub and the full tests:

```rust
//! F3 prerequisite — one-shot SHA-256 (FIPS 180-4), hand-rolled because the
//! hash runs at startup (asset↔sidecar binding) and the crate has a
//! zero-runtime-dependency rule: it must keep building offline on the Pi.
//! One-shot only — the payload is always fully in memory — so there is no
//! streaming state to get wrong. Correctness is pinned by the NIST vectors
//! (including the one-million-'a' vector) plus shasum-verified inputs at
//! every padding boundary.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
    0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
    0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
    0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
    0x5be0cd19,
];

/// SHA-256 of `data`, one shot.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let _ = data;
    [0u8; 32] // STUB — replaced in step 3
}

/// SHA-256 of `data` as 64 lowercase hex characters — the sidecar format.
pub fn sha256_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let d = sha256(data);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_times(n: usize) -> Vec<u8> {
        vec![b'a'; n]
    }

    #[test]
    fn nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    // The NIST long vector: one million 'a'. Exercises many blocks and the
    // rem == 0 padding path on a large input. Milliseconds even in debug.
    #[test]
    fn nist_million_a() {
        assert_eq!(
            sha256_hex(&a_times(1_000_000)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    // Padding boundaries: 55 is the last 1-padding-block length, 56 the first
    // 2-block one; 63/64/65 straddle the block size; 112 covers a mid-size
    // rem. Reference digests generated with `shasum -a 256` on 2026-08-15.
    #[test]
    fn padding_boundaries() {
        for (n, want) in [
            (55, "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"),
            (56, "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"),
            (63, "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34"),
            (64, "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"),
            (65, "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0"),
            (112, "f54353008a2553262ecdc4a34749563ba0950e8b0fc8652780b0a614b99683c1"),
        ] {
            assert_eq!(sha256_hex(&a_times(n)), want, "length {n}");
        }
    }
}
```

**Step 2 (1 min).** In `src/lib.rs` add `pub mod sha256;` after `pub mod ffi_consts;`.
Run `cargo test --lib` — the four sha256 tests FAIL (stub). Red confirmed.

**Step 3 (5 min).** Replace the stub `sha256` body with the real implementation and add
the private `compress` below it:

```rust
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let mut chunks = data.chunks_exact(64);
    for block in &mut chunks {
        compress(&mut state, block.try_into().unwrap());
    }

    // Padding: 0x80, zeros, and the bit length as a big-endian u64 — either
    // one or two final blocks depending on how much room the remainder left.
    let rem = chunks.remainder();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut last = [0u8; 128];
    last[..rem.len()].copy_from_slice(rem);
    last[rem.len()] = 0x80;
    let blocks = if rem.len() + 1 + 8 <= 64 { 1 } else { 2 };
    let total = blocks * 64;
    last[total - 8..total].copy_from_slice(&bitlen.to_be_bytes());
    for i in 0..blocks {
        compress(&mut state, (&last[i * 64..(i + 1) * 64]).try_into().unwrap());
    }

    let mut out = [0u8; 32];
    for (i, s) in state.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&s.to_be_bytes());
    }
    out
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[4 * i],
            block[4 * i + 1],
            block[4 * i + 2],
            block[4 * i + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}
```

**Step 4 (1 min).** `cargo test --lib` — green. `cargo check --all-targets` — clean.

**Step 5 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/sha256.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs
git commit -m "E: 4k-loop-probe — F3 prep: zero-dependency SHA-256 with NIST + boundary vectors"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 4 — Failure-path CLI harness + the never-hangs regression (T3)

**Files.** Create `tests/cli.rs`. Runs on the Pi (`cargo test`) — it spawns the real
binary. Type-checks on the Mac via `cargo check --all-targets`.

**Interfaces.**
Consumes: the built binary via `env!("CARGO_BIN_EXE_dex-loop")`;
`dex_loop::sha256::sha256_hex` (for `write_sidecar` — inert until task 6, binding after).
Produces (test-internal, reused by tasks 6–8): `run_with_deadline(&[&str], Duration) ->
Run`, `Run { exit_code: Option<i32>, stderr: String }`, `temp_path(&str) -> PathBuf`,
`stub_annexb() -> Vec<u8>`, `write_sidecar(&Path, &[u8], &str)`, `GATE_EXIT`,
`RUNTIME_EXIT`.

**Step 1 (10 min).** Create `tests/cli.rs`:

```rust
//! T3 — failure-path integration tests. Every serious bug in this program
//! lived in a failure path; the happy path was never the problem. Each test
//! asserts EXIT BEHAVIOUR (code + boundedness), not output niceties.
//!
//! These tests spawn the real binary, which links libmpv — this target runs
//! on the Pi (`cargo test`); on the Mac use `cargo test --lib`.
//!
//! DISPLAY SAFETY: a soak may own the display. Every invocation that can
//! reach mpv_create MUST carry `--no-defaults --opt vo=null --opt vid=no
//! --opt aid=no`: the null VO never touches DRM, and deselecting all tracks
//! makes mpv end deterministically (NOTHING_TO_PLAY -> END_FILE) instead of
//! playing forever.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Exit-code contract (see usage()): refused before playback vs runtime failure.
const GATE_EXIT: i32 = 2;
const RUNTIME_EXIT: i32 = 1;

/// Outcome of one run. `exit_code` is None when the process had to be killed
/// at the deadline OR died by signal — both are failures the assertions catch.
struct Run {
    exit_code: Option<i32>,
    stderr: String,
}

/// Spawn dex-loop, wait at most `deadline`, kill on overrun. THE DEADLINE IS
/// THE ASSERTION: a player that hangs on a failure path is this program's
/// worst outcome — alive, supervisor green, screen black. The shipped
/// END_FILE idle-hang was exactly that.
fn run_with_deadline(args: &[&str], deadline: Duration) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dex-loop"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dex-loop");
    // Drain stderr on a thread so a chatty child can never fill the pipe and
    // block — a blocked child would masquerade as a hang.
    let mut pipe = child.stderr.take().expect("stderr piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        pipe.read_to_string(&mut s).ok();
        s
    });
    let start = Instant::now();
    let exit_code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code(),
            None if start.elapsed() > deadline => {
                child.kill().ok();
                child.wait().ok();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stderr = reader.join().expect("stderr reader");
    Run { exit_code, stderr }
}

/// Unique-per-test scratch path (std::env::temp_dir; no cleanup needed).
fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("dex-loop-test-{}-{}", std::process::id(), name));
    p
}

/// Minimal Annex-B HEVC scaffold: VPS, SPS, PPS, then an IDR_W_RADL slice.
/// Structurally what the F4 NAL gate requires; NOT decodable video — these
/// tests assert exit behaviour, never pixels.
fn stub_annexb() -> Vec<u8> {
    fn nal(nal_type: u8, payload: &[u8]) -> Vec<u8> {
        // 4-byte start code + 2-byte NAL header (forbidden=0, layer=0, tid+1=1)
        let mut v = vec![0, 0, 0, 1, nal_type << 1, 0x01];
        v.extend_from_slice(payload);
        v
    }
    let mut v = Vec::new();
    v.extend(nal(32, &[0x2a; 8])); // VPS
    v.extend(nal(33, &[0x2a; 16])); // SPS
    v.extend(nal(34, &[0x2a; 4])); // PPS
    v.extend(nal(19, &[0x2a; 64])); // IDR_W_RADL slice
    v
}

/// Write `<asset>.json` binding `bytes` at `fps` — the deploy-path fixture.
/// Inert before task 6 (the player ignores it); binding afterwards.
fn write_sidecar(asset: &Path, bytes: &[u8], fps: &str) {
    let sha = dex_loop::sha256::sha256_hex(bytes);
    std::fs::write(
        format!("{}.json", asset.display()),
        format!(r#"{{"fps":"{fps}","sha256":"{sha}"}}"#),
    )
    .unwrap();
}

#[test]
fn missing_file_exits_2_and_names_the_path() {
    let r = run_with_deadline(
        &["/nonexistent/dex-loop-test.265", "--fps", "30"],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("/nonexistent/dex-loop-test.265"),
        "stderr must name the path: {}",
        r.stderr
    );
}

#[test]
fn empty_file_exits_2() {
    let p = temp_path("empty.265");
    std::fs::write(&p, b"").unwrap();
    let r = run_with_deadline(&[p.to_str().unwrap(), "--fps", "30"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
}

#[test]
fn no_args_exits_2_with_usage() {
    let r = run_with_deadline(&[], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT));
    assert!(r.stderr.contains("usage"), "stderr: {}", r.stderr);
}

/// THE regression for shipped bug #1: mpv_create enables idle mode, so a
/// playback failure emits END_FILE and then idles FOREVER unless the event
/// loop treats END_FILE as fatal. Force a deterministic, display-free
/// playback failure (vid=no + aid=no deselect every track -> mpv ends with
/// "nothing to play" -> END_FILE) and assert the process EXITS, code 1,
/// within the deadline. Before the END_FILE fix this exact scenario sat in
/// idle indefinitely with the supervisor reading green.
#[test]
fn playback_failure_exits_nonzero_never_hangs() {
    let p = temp_path("stub.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert!(
        r.exit_code.is_some(),
        "player HUNG on a playback failure (killed at deadline); stderr: {}",
        r.stderr
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
}

/// Undecodable garbage must produce a BOUNDED, nonzero exit — never a hang.
/// Today the garbage reaches mpv and fails its (bounded) demux probe
/// (LOADING_FAILED -> END_FILE -> exit 1). Once the F4 NAL gate lands, the
/// same input is refused before mpv starts (exit 2); task 7 tightens this
/// assertion to exactly that.
#[test]
fn garbage_bytes_exit_nonzero_within_deadline() {
    let p = temp_path("garbage.265");
    // 64 KiB of bytes in 0x02..=0x7E: no 0x00/0x01 (no Annex-B start code
    // anywhere) and no 0xFF (no MP3/ADTS sync word a demuxer could latch onto).
    let bytes: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761) >> 24) as u8 % 0x7d) + 0x02)
        .collect();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert!(
        r.exit_code.is_some(),
        "player HUNG on garbage input; stderr: {}",
        r.stderr
    );
    assert_ne!(r.exit_code, Some(0), "stderr: {}", r.stderr);
}
```

**Step 2 (1 min).** Mac: `cargo check --all-targets` — type-checks (no link).

**Step 3 (4 min).** Pi: rsync + `nice -n 19 cargo test` per §2. All five tests pass —
these are baseline/regression tests of already-fixed behaviour (that is their red-phase
substitute; the real red is step 4).

**Step 4 (6 min).** Prove the never-hangs test bites. In `src/main.rs`, temporarily
replace the END_FILE branch body — the block starting `if id == MPV_EVENT_END_FILE {` —
so it reads `if id == MPV_EVENT_END_FILE { continue; }` (re-creating bug #1's idle-hang).
Rsync, run on the Pi: `playback_failure_exits_nonzero_never_hangs` must FAIL after ~30 s
with "player HUNG". Revert the edit (`git -C /Users/mfa/CODE/dex checkout -- experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs`),
rsync, green again.

**Step 5 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/tests/cli.rs
git commit -m "E: 4k-loop-probe — T3: failure-path CLI harness incl. never-hangs END_FILE regression"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 5 — Sidecar parsing + fps binding rules (F3, pure logic)

**Files.** Create `src/sidecar.rs`. Modify `src/lib.rs`. Tests inside the module, Mac.

**Interfaces.**
Produces: `dex_loop::sidecar::{Value, parse_flat_json, Sidecar, is_valid_fps, FpsSource,
resolve_fps, verify_payload}` (§2). Consumes: `crate::sha256::sha256_hex`.

**Step 1 (12 min).** Create `src/sidecar.rs`. Public items as stubs (`todo!()`), full
docs and tests:

```rust
//! F3 — the asset+fps sidecar: parse, validate, and decide the binding.
//!
//! Why: the one failure that is undetectable BY CONSTRUCTION. A raw Annex-B
//! stream has no timestamps, so `--fps 25` on a 30 fps asset plays 20% slow,
//! forever, with zero errors and every metric nominal. The fix: the frame
//! rate travels WITH the asset (a sidecar written at ingest), bound by a
//! sha256 so a stale/wrong/truncated asset is refused at startup.
//!
//! Format — `<asset>.json` next to the asset (`loop.265` -> `loop.265.json`):
//!
//! ```json
//! {"fps":"30","sha256":"<64 hex>","width":3840,"height":2160,
//!  "source":"card.mp4","encoder_cmd":"ffmpeg ..."}
//! ```
//!
//! `fps` is a STRING, not a JSON number: "30000/1001" must survive exactly,
//! and 29.97 as a float invites drift. It is passed verbatim to mpv's
//! container-fps-override after grammar validation. Required: fps, sha256.
//! Optional, informational: width, height (integers), source, encoder_cmd
//! (strings). Unknown keys are ignored so ingest can add metadata without
//! breaking deployed players.
//!
//! The parser accepts a STRICT SUBSET of JSON — one flat object, string and
//! unsigned-integer values, escapes \" \\ \/ \n \r \t only. Anything else is
//! a parse error, and a parse error refuses startup. Fail-closed IS the F3
//! semantics: an unparseable sidecar and a missing one are the same
//! operational fact. Hand-rolled because the crate has a zero-dependency rule
//! and must build offline on the Pi.

/// A value in the sidecar subset: strings and unsigned integers only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Num(u64),
}

/// Parse the strict flat-object JSON subset. Returns key/value pairs in
/// document order. Duplicate keys are an error — a hand-edited sidecar with
/// two `fps` lines must not silently pick one.
pub fn parse_flat_json(text: &str) -> Result<Vec<(String, Value)>, String> {
    let _ = text;
    todo!() // STUB — replaced in step 3
}

/// The parsed, validated sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    /// Verbatim frame rate for mpv's container-fps-override:
    /// "30", "29.97", or "30000/1001".
    pub fps: String,
    /// Lowercase 64-hex-digit SHA-256 of the asset bytes.
    pub sha256: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

/// Accepts: integer "30", decimal "29.97", rational "30000/1001".
/// Rejects: empty, zero ("0", "00", "0.0", "0/x", "x/0" — a zero rate is
/// always a typo), signs, spaces, exponents, dangling '.' or '/'.
pub fn is_valid_fps(s: &str) -> bool {
    let _ = s;
    todo!() // STUB — replaced in step 3
}

impl Sidecar {
    /// Parse and validate sidecar JSON. Every error refuses startup (fail
    /// closed); messages are written for the journal, naming what to fix.
    pub fn from_json(text: &str) -> Result<Sidecar, String> {
        let _ = text;
        todo!() // STUB — replaced in step 3
    }
}

/// Where the effective fps came from — logged at startup so a bench override
/// is always visible in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsSource {
    Sidecar,
    BenchOverride,
}

/// Decide the effective fps from (sidecar fps, --fps, --bench-no-sidecar).
///
/// The rules make operator error impossible rather than detectable (there is
/// no operator):
/// - deploy path: sidecar present, no flags -> sidecar fps.
/// - sidecar + --fps: allowed only when EXACTLY equal (string equality); a
///   stale wrapper script must fail loudly, never win silently.
/// - no sidecar: refused — unless --bench-no-sidecar AND --fps are BOTH
///   given. The bench escape hatch is a deliberate two-flag act.
pub fn resolve_fps(
    sidecar_fps: Option<&str>,
    cli_fps: Option<&str>,
    bench_no_sidecar: bool,
) -> Result<(String, FpsSource), String> {
    let _ = (sidecar_fps, cli_fps, bench_no_sidecar);
    todo!() // STUB — replaced in step 3
}

/// Bind the asset bytes to the sidecar: one hash pass over the payload the
/// player already holds in memory. Truncated copies, wrong files and stale
/// sidecars all land here.
pub fn verify_payload(payload: &[u8], sidecar: &Sidecar) -> Result<(), String> {
    let _ = (payload, sidecar);
    todo!() // STUB — replaced in step 3
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn parses_the_canonical_sidecar() {
        let text = format!(
            r#"{{"fps":"30","sha256":"{GOOD_SHA}","width":3840,"height":2160,"source":"card.mp4","encoder_cmd":"ffmpeg -i card.mp4 -c:v copy"}}"#
        );
        let s = Sidecar::from_json(&text).unwrap();
        assert_eq!(s.fps, "30");
        assert_eq!(s.sha256, GOOD_SHA);
        assert_eq!(s.width, Some(3840));
        assert_eq!(s.height, Some(2160));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = format!(r#"{{"fps":"30","sha256":"{GOOD_SHA}","future_key":"whatever"}}"#);
        assert!(Sidecar::from_json(&text).is_ok());
    }

    #[test]
    fn fps_as_number_is_refused_with_guidance() {
        let text = format!(r#"{{"fps":30,"sha256":"{GOOD_SHA}"}}"#);
        let e = Sidecar::from_json(&text).unwrap_err();
        assert!(e.contains("STRING"), "{e}");
    }

    #[test]
    fn missing_required_keys_are_refused_by_name() {
        let e = Sidecar::from_json(&format!(r#"{{"sha256":"{GOOD_SHA}"}}"#)).unwrap_err();
        assert!(e.contains("fps"), "{e}");
        let e = Sidecar::from_json(r#"{"fps":"30"}"#).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }

    #[test]
    fn bad_sha256_is_refused_uppercase_is_normalized() {
        for sha in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            let text = format!(r#"{{"fps":"30","sha256":"{sha}"}}"#);
            assert!(Sidecar::from_json(&text).is_err(), "sha {sha:?} accepted");
        }
        let text = format!(r#"{{"fps":"30","sha256":"{}"}}"#, GOOD_SHA.to_uppercase());
        assert_eq!(Sidecar::from_json(&text).unwrap().sha256, GOOD_SHA);
    }

    #[test]
    fn fps_grammar() {
        for ok in ["30", "25", "29.97", "23.976", "30000/1001", "60"] {
            assert!(is_valid_fps(ok), "{ok} should be valid");
        }
        for bad in [
            "", "0", "00", "0/30", "30/0", "-30", "+30", " 30", "30 ", "30/", "/1001",
            "1e3", "29.97.5", "29.", ".97", "banana", "0.0",
        ] {
            assert!(!is_valid_fps(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn parser_rejects_everything_outside_the_subset() {
        for bad in [
            "",                             // no object
            "[1,2]",                        // array at top level
            r#"{"a":{"b":1}}"#,             // nested object
            r#"{"a":[1]}"#,                 // array value
            r#"{"a":true}"#,                // boolean
            r#"{"a":null}"#,                // null
            r#"{"a":-1}"#,                  // negative number
            r#"{"a":1.5}"#,                 // float
            r#"{"a":1e3}"#,                 // exponent
            r#"{"a":"x"}"trailing"#,        // trailing data
            r#"{"a":"x""b":"y"}"#,          // missing comma
            r#"{"a":"unterminated}"#,       // unterminated string
            "{\"a\":\"bad \\u0041 escape\"}", // \u outside the subset
            r#"{"a":"x","a":"y"}"#,         // duplicate key
        ] {
            assert!(parse_flat_json(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn parser_accepts_the_subset() {
        let kv = parse_flat_json(r#" { "a" : "x\n\"q\"" , "n" : 42 } "#).unwrap();
        assert_eq!(
            kv,
            vec![
                ("a".to_string(), Value::Str("x\n\"q\"".to_string())),
                ("n".to_string(), Value::Num(42)),
            ]
        );
        assert_eq!(parse_flat_json("{}").unwrap(), vec![]);
    }

    #[test]
    fn deploy_path_takes_fps_from_sidecar() {
        assert_eq!(
            resolve_fps(Some("30"), None, false).unwrap(),
            ("30".to_string(), FpsSource::Sidecar)
        );
    }

    #[test]
    fn agreeing_cli_fps_allowed_disagreeing_refused_naming_both() {
        assert!(resolve_fps(Some("30"), Some("30"), false).is_ok());
        let e = resolve_fps(Some("30"), Some("25"), false).unwrap_err();
        assert!(e.contains("30") && e.contains("25"), "{e}");
    }

    #[test]
    fn missing_sidecar_is_refused_without_the_bench_flag() {
        let e = resolve_fps(None, Some("30"), false).unwrap_err();
        assert!(e.contains("bench"), "{e}");
        assert!(resolve_fps(None, None, false).is_err());
    }

    #[test]
    fn bench_escape_hatch_requires_both_flags_and_a_valid_rate() {
        assert_eq!(
            resolve_fps(None, Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        // bench flag with a sidecar present: the sidecar is IGNORED — that is
        // what "bench" means — and the CLI value wins.
        assert_eq!(
            resolve_fps(Some("25"), Some("30"), true).unwrap(),
            ("30".to_string(), FpsSource::BenchOverride)
        );
        assert!(resolve_fps(None, None, true).is_err());
        assert!(resolve_fps(None, Some("banana"), true).is_err());
    }

    #[test]
    fn verify_payload_binds_bytes_to_sidecar() {
        let payload = b"the asset bytes";
        let s = Sidecar {
            fps: "30".into(),
            sha256: crate::sha256::sha256_hex(payload),
            width: None,
            height: None,
        };
        assert!(verify_payload(payload, &s).is_ok());
        let bad = Sidecar {
            fps: "30".into(),
            sha256: "a".repeat(64),
            width: None,
            height: None,
        };
        let e = verify_payload(payload, &bad).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }
}
```

**Step 2 (1 min).** In `src/lib.rs` add `pub mod sidecar;` after `pub mod sha256;`.
`cargo test --lib` — the sidecar tests FAIL (todo! panics). Red confirmed.

**Step 3 (15 min).** Replace the five stub bodies. `parse_flat_json` plus its private
parser:

```rust
pub fn parse_flat_json(text: &str) -> Result<Vec<(String, Value)>, String> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    p.skip_ws();
    p.expect(b'{')?;
    let mut out: Vec<(String, Value)> = Vec::new();
    p.skip_ws();
    if p.peek() == Some(b'}') {
        p.i += 1;
    } else {
        loop {
            p.skip_ws();
            let key = p.string()?;
            if out.iter().any(|(k, _)| *k == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            p.skip_ws();
            p.expect(b':')?;
            p.skip_ws();
            let val = match p.peek() {
                Some(b'"') => Value::Str(p.string()?),
                Some(c) if c.is_ascii_digit() => Value::Num(p.number()?),
                Some(c) => {
                    return Err(format!(
                        "unsupported value starting with {:?} (subset: strings and unsigned integers only)",
                        c as char
                    ))
                }
                None => return Err("unexpected end of input".into()),
            };
            out.push((key, val));
            p.skip_ws();
            match p.next_byte() {
                Some(b',') => continue,
                Some(b'}') => break,
                other => return Err(format!("expected ',' or '}}', got {other:?}")),
            }
        }
    }
    p.skip_ws();
    if p.i != p.b.len() {
        return Err("trailing data after closing '}'".into());
    }
    Ok(out)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn next_byte(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        match self.next_byte() {
            Some(g) if g == c => Ok(()),
            g => Err(format!("expected {:?}, got {g:?}", c as char)),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s: Vec<u8> = Vec::new();
        loop {
            match self.next_byte() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    return String::from_utf8(s)
                        .map_err(|_| String::from("invalid UTF-8 in string"))
                }
                Some(b'\\') => match self.next_byte() {
                    Some(b'"') => s.push(b'"'),
                    Some(b'\\') => s.push(b'\\'),
                    Some(b'/') => s.push(b'/'),
                    Some(b'n') => s.push(b'\n'),
                    Some(b'r') => s.push(b'\r'),
                    Some(b't') => s.push(b'\t'),
                    e => {
                        return Err(format!(
                            r#"unsupported escape {e:?} (subset: \" \\ \/ \n \r \t)"#
                        ))
                    }
                },
                Some(c) if c < 0x20 => return Err("raw control character in string".into()),
                // Multibyte UTF-8 passes through byte-wise: continuation bytes
                // are >= 0x80, so they can never be mistaken for '"' or '\\'.
                Some(c) => s.push(c),
            }
        }
    }
    fn number(&mut self) -> Result<u64, String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return Err("expected digits".into());
        }
        // Reject the rest of JSON's number grammar explicitly: the subset is
        // unsigned integers only (write fps as a string).
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err("floats are outside the sidecar subset (write fps as a string)".into());
        }
        std::str::from_utf8(&self.b[start..self.i])
            .unwrap()
            .parse::<u64>()
            .map_err(|e| format!("number out of range: {e}"))
    }
}
```

`is_valid_fps`:

```rust
pub fn is_valid_fps(s: &str) -> bool {
    fn positive_int(t: &str) -> bool {
        !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && t.bytes().any(|b| b != b'0')
    }
    if let Some((num, den)) = s.split_once('/') {
        return positive_int(num) && positive_int(den);
    }
    if let Some((int, frac)) = s.split_once('.') {
        let digits_ok = !int.is_empty()
            && int.bytes().all(|b| b.is_ascii_digit())
            && !frac.is_empty()
            && frac.bytes().all(|b| b.is_ascii_digit());
        let nonzero = int.bytes().chain(frac.bytes()).any(|b| b != b'0');
        return digits_ok && nonzero;
    }
    positive_int(s)
}
```

`Sidecar::from_json`:

```rust
    pub fn from_json(text: &str) -> Result<Sidecar, String> {
        let kv = parse_flat_json(text).map_err(|e| format!("sidecar JSON: {e}"))?;
        let mut fps = None;
        let mut sha = None;
        let mut width = None;
        let mut height = None;
        for (k, v) in kv {
            match (k.as_str(), v) {
                ("fps", Value::Str(s)) => fps = Some(s),
                ("fps", Value::Num(_)) => {
                    return Err("sidecar: fps must be a JSON STRING (\"30\", \"30000/1001\") \
                                so rational rates survive exactly"
                        .into())
                }
                ("sha256", Value::Str(s)) => sha = Some(s),
                ("sha256", Value::Num(_)) => {
                    return Err("sidecar: sha256 must be a string".into())
                }
                ("width", Value::Num(n)) => width = Some(n),
                ("height", Value::Num(n)) => height = Some(n),
                ("width", Value::Str(_)) | ("height", Value::Str(_)) => {
                    return Err("sidecar: width/height must be integers".into())
                }
                // Unknown keys and informational strings: ignored, so ingest
                // can add metadata without breaking deployed players.
                _ => {}
            }
        }
        let fps = fps.ok_or("sidecar: missing required key \"fps\"")?;
        if !is_valid_fps(&fps) {
            return Err(format!(
                "sidecar: invalid fps {fps:?} (expect \"30\", \"29.97\" or \"30000/1001\")"
            ));
        }
        let sha = sha.ok_or("sidecar: missing required key \"sha256\"")?;
        let sha = sha.to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("sidecar: sha256 must be 64 hex digits, got {sha:?}"));
        }
        Ok(Sidecar { fps, sha256: sha, width, height })
    }
```

`resolve_fps`:

```rust
pub fn resolve_fps(
    sidecar_fps: Option<&str>,
    cli_fps: Option<&str>,
    bench_no_sidecar: bool,
) -> Result<(String, FpsSource), String> {
    if bench_no_sidecar {
        return match cli_fps {
            Some(f) if is_valid_fps(f) => Ok((f.to_string(), FpsSource::BenchOverride)),
            Some(f) => Err(format!("--fps {f:?} is not a valid frame rate")),
            None => Err("--bench-no-sidecar requires an explicit --fps".into()),
        };
    }
    match (sidecar_fps, cli_fps) {
        (Some(s), None) => Ok((s.to_string(), FpsSource::Sidecar)),
        (Some(s), Some(c)) if s == c => Ok((s.to_string(), FpsSource::Sidecar)),
        (Some(s), Some(c)) => Err(format!(
            "--fps {c} contradicts sidecar fps {s}; drop --fps (the sidecar is authoritative) \
             or fix the sidecar"
        )),
        (None, _) => Err(
            "no sidecar found; refusing to guess the frame rate. Re-ingest the asset to \
             produce <asset>.json, or use --bench-no-sidecar --fps <F> on a bench"
                .into(),
        ),
    }
}
```

`verify_payload`:

```rust
pub fn verify_payload(payload: &[u8], sidecar: &Sidecar) -> Result<(), String> {
    let actual = crate::sha256::sha256_hex(payload);
    if actual != sidecar.sha256 {
        return Err(format!(
            "asset does not match its sidecar: sha256 {actual} != sidecar {}; the asset or \
             sidecar is stale, wrong, or truncated — re-ingest",
            sidecar.sha256
        ));
    }
    Ok(())
}
```

**Step 4 (1 min).** `cargo test --lib` — all 13 sidecar tests green.
`cargo check --all-targets` — clean.

**Step 5 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/sidecar.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs
git commit -m "E: 4k-loop-probe — F3: sidecar subset parser, fps grammar, binding rules (pure)"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 6 — Wire the sidecar gate into the player (F3)

**Files.** Modify `src/main.rs`, `tests/cli.rs`. Test on the Pi.

**Interfaces.**
Consumes: `dex_loop::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar}`;
test fixtures from task 4 (`run_with_deadline`, `temp_path`, `stub_annexb`,
`write_sidecar`, `GATE_EXIT`, `RUNTIME_EXIT`).
Produces: the CLI contract — sidecar `<asset>.json` required; `--fps` optional
cross-check; `--bench-no-sidecar --fps F` escape hatch; refusals exit 2.

**Step 1 (8 min).** Append the failing tests to `tests/cli.rs`:

```rust
// ---- F3: sidecar binding (task 6) ---------------------------------------

#[test]
fn missing_sidecar_refused_exit_2_naming_the_sidecar_path() {
    let p = temp_path("nosidecar.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let _ = std::fs::remove_file(format!("{}.json", p.display()));
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains(&format!("{}.json", p.display())),
        "stderr must name the sidecar path: {}",
        r.stderr
    );
}

/// The truncated-copy case F4 can NEVER catch: leading NALs intact, tail
/// missing. Only the hash sees it. The sidecar binds the FULL bytes; the
/// file on disk is truncated.
#[test]
fn truncated_asset_vs_full_hash_refused_exit_2() {
    let p = temp_path("truncated.265");
    let full = stub_annexb();
    write_sidecar(&p, &full, "30");
    std::fs::write(&p, &full[..full.len() - 20]).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("sha256"), "stderr: {}", r.stderr);
}

#[test]
fn fps_contradicting_sidecar_refused_exit_2_naming_both() {
    let p = temp_path("fpsconflict.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "25",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("25") && r.stderr.contains("30"),
        "stderr must name both rates: {}",
        r.stderr
    );
}

/// Exit 1 — the mpv path — proves the gates PASSED with an agreeing --fps.
#[test]
fn agreeing_fps_and_sidecar_reach_playback() {
    let p = temp_path("fpsagree.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
}

/// Exit 1, not 2: the two-flag bench escape hatch bypasses the sidecar gate
/// and reaches playback with no sidecar on disk.
#[test]
fn bench_escape_hatch_bypasses_sidecar() {
    let p = temp_path("bench.265");
    std::fs::write(&p, stub_annexb()).unwrap(); // deliberately no sidecar
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
            "--fps",
            "30",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
}

#[test]
fn bench_flag_without_fps_refused_exit_2() {
    let p = temp_path("benchnofps.265");
    std::fs::write(&p, stub_annexb()).unwrap();
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--bench-no-sidecar",
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
}
```

**Step 2 (3 min).** Red: rsync + Pi `cargo test`. Expect FAILURES:
`missing_sidecar...` (old binary prints usage, not the sidecar path),
`truncated...` (old binary plays the truncated stub → exit 1),
`fps_contradicting...` (old binary plays at 25 → exit 1),
`bench_escape_hatch...` (old binary rejects the unknown flag → exit 2).
`agreeing_fps...` and `bench_flag_without_fps...` may pass already (exit codes
coincide) — fine; they pin behaviour across the change.

**Step 3 (10 min).** Implement in `src/main.rs`:

(a) After the `use dex_loop::ffi_consts::...;` block, add:

```rust
use dex_loop::sidecar::{resolve_fps, verify_payload, FpsSource, Sidecar};
```

(b) Replace the whole `fn usage() -> !` with:

```rust
fn usage() -> ! {
    eprintln!(
        "usage: dex-loop <stream.265> [--fps <F>] [--mode WxH@R] [--bench-no-sidecar] [--no-defaults] [--opt K=V ...]

  <stream.265>        raw Annex-B HEVC elementary stream, looped endlessly
  <stream.265>.json   ingest sidecar, REQUIRED: {{\"fps\":\"30\",\"sha256\":\"<64 hex>\"}}
                      fps comes from it; the sha256 must match the asset bytes
  --fps F             optional cross-check; must equal the sidecar fps exactly
  --mode WxH@R        force a DRM mode, e.g. 3840x2160@30 (default: connector preferred)
  --bench-no-sidecar  BENCH ONLY: skip the sidecar, take --fps as given
  --opt K=V           pass an extra mpv option (repeatable)
  --no-defaults       omit the built-in Pi 4 zero-copy option set

exit codes: 2 = refused before playback (bad invocation/asset/sidecar; fix and redeploy)
            1 = playback/runtime failure (the supervisor restarts)"
    );
    std::process::exit(2)
}
```

(c) In `main()`, replace the declaration `let mut fps: Option<String> = None;` with
`let mut cli_fps: Option<String> = None;`, and in the match arm `"--fps" => { ... }`
replace `fps = args.get(i).cloned();` with `cli_fps = args.get(i).cloned();`. After the
`"--no-defaults" => defaults = false,` arm add:

```rust
            "--bench-no-sidecar" => bench_no_sidecar = true,
```

and after `let mut defaults = true;` add:

```rust
    let mut bench_no_sidecar = false;
```

(d) Replace

```rust
    let (Some(path), Some(fps)) = (path, fps) else {
        usage()
    };
```

with

```rust
    let Some(path) = path else { usage() };
```

(e) Immediately after the `let leaked: &'static [u8] = Box::leak(...)` line, DELETE the
old `eprintln!("dex-loop: {} bytes, looping endlessly", leaked.len());` line and insert:

```rust
    // F3 — bind the asset to its ingest sidecar. A raw Annex-B stream has no
    // timestamps: a WRONG --fps plays slow/fast forever with zero errors and
    // every metric nominal — the one failure that is undetectable by
    // construction. So the frame rate travels WITH the asset, bound by a
    // sha256, and an unbound asset is refused. `--bench-no-sidecar --fps F`
    // is the deliberate two-flag bench escape hatch.
    let sidecar_path = format!("{path}.json");
    let sidecar: Option<Sidecar> = if bench_no_sidecar {
        None
    } else {
        match fs::read_to_string(&sidecar_path) {
            Ok(text) => match Sidecar::from_json(&text) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("error: {sidecar_path}: {e}");
                    return ExitCode::from(2);
                }
            },
            Err(e) => {
                eprintln!(
                    "error: cannot read sidecar {sidecar_path}: {e}\n\
                     an asset without its ingest sidecar is unbound (fps would be a \
                     guess); re-ingest to produce it, or use --bench-no-sidecar \
                     --fps <F> on a bench"
                );
                return ExitCode::from(2);
            }
        }
    };

    let (fps, fps_source) = match resolve_fps(
        sidecar.as_ref().map(|s| s.fps.as_str()),
        cli_fps.as_deref(),
        bench_no_sidecar,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    if let Some(s) = &sidecar {
        if let Err(e) = verify_payload(leaked, s) {
            eprintln!("error: {path}: {e}");
            return ExitCode::from(2);
        }
    }

    eprintln!(
        "dex-loop: {} bytes, fps {fps} ({}), looping endlessly",
        leaked.len(),
        match fps_source {
            FpsSource::Sidecar => "sidecar",
            FpsSource::BenchOverride => "BENCH OVERRIDE, unbound",
        }
    );
```

The later line `opts.push(("container-fps-override".into(), fps));` keeps working — `fps`
is now the resolved `String`.

**Step 4 (3 min).** Mac `cargo check --all-targets && cargo test --lib`; then rsync +
Pi `cargo test` — everything green, including all six new tests and the five task-4
tests (whose fixtures already wrote agreeing sidecars).

**Step 5 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/tests/cli.rs
git commit -m "E: 4k-loop-probe — F3: sidecar gate wired; --fps is a cross-check, bench needs two flags"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 7 — Leading-NAL validation gate (F4)

**Files.** Create `src/nal.rs`. Modify `src/lib.rs`, `src/main.rs`, `tests/cli.rs`.

**Interfaces.**
Produces: `dex_loop::nal::{validate_leading_nals, NAL_VPS, NAL_SPS, NAL_PPS,
NAL_IDR_W_RADL, NAL_IDR_N_LP}` (§2). Consumed by `main()` after `verify_payload`.

**Step 1 (10 min).** Create `src/nal.rs` with a stub and full tests:

```rust
//! F4 — the asset validation gate: refuse an asset whose leading NALs cannot
//! support the gaplessness premise.
//!
//! The endless-stream design only wraps seamlessly because byte 0 begins a
//! closed GOP: parameter sets (VPS/SPS/PPS) then an IDR, so re-entering at
//! byte 0 mid-stream is an ordinary keyframe, not a seek. An asset that
//! starts with anything else — an open-GOP CRA, a trailing slice, no
//! parameter sets — would "play" and then glitch at EVERY wrap (~29k visible
//! artefacts/day for a 3 s loop), silently. The hash (F3) proves the bytes
//! are the ingested bytes; this gate proves the ingested bytes have the
//! required SHAPE. Truncation is F3's job: a truncated copy has intact
//! leading NALs and passes this gate by design.
//!
//! Why IDR only (19/20), not any IRAP (16-23): CRA (21) admits RASL leading
//! pictures whose wrap-join correctness depends on the content; BLA (16-18)
//! never comes from a sane ingest; 22/23 are reserved. The premise stated
//! everywhere in this crate is "IDR at frame 0" — so that is what the gate
//! enforces. Relax knowingly if an asset ever justifies it.

/// HEVC nal_unit_type values (ITU-T H.265 Table 7-1) this gate names.
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;
pub const NAL_IDR_W_RADL: u8 = 19;
pub const NAL_IDR_N_LP: u8 = 20;

/// Validate the leading NAL units of a raw Annex-B HEVC stream.
///
/// Passes iff, before the first VCL NAL (type 0-31), all of VPS/SPS/PPS have
/// appeared, and that first VCL NAL is an IDR (19 or 20). Everything after
/// the first VCL NAL is out of scope — the F3 hash covers byte-level
/// integrity of the whole asset.
///
/// Scanning is a plain 00 00 01 search (3- and 4-byte start codes both
/// resolve to it): encoders insert emulation-prevention bytes precisely so
/// that pattern never occurs inside a NAL payload, so the search cannot
/// false-positive on a well-formed stream.
pub fn validate_leading_nals(data: &[u8]) -> Result<(), String> {
    let _ = data;
    todo!() // STUB — replaced in step 3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One NAL: 4-byte start code + 2-byte header (layer 0, tid+1 = 1).
    fn nal(nal_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, nal_type << 1, 0x01];
        v.extend_from_slice(payload);
        v
    }

    fn stream(types: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        for &t in types {
            v.extend(nal(t, &[0x2a; 8]));
        }
        v
    }

    #[test]
    fn valid_closed_gop_asset_passes() {
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 19])).is_ok()); // IDR_W_RADL
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 20])).is_ok()); // IDR_N_LP
        // non-VCL noise before/among parameter sets is fine (AUD=35, SEI=39)
        assert!(validate_leading_nals(&stream(&[35, 32, 39, 33, 34, 19])).is_ok());
        // trailing slices after the IDR are out of scope for the gate
        assert!(validate_leading_nals(&stream(&[32, 33, 34, 19, 1, 0, 1])).is_ok());
    }

    #[test]
    fn three_byte_start_codes_pass_too() {
        let mut v = Vec::new();
        for t in [32u8, 33, 34, 19] {
            v.extend([0, 0, 1, t << 1, 0x01]);
            v.extend([0x2a; 8]);
        }
        assert!(validate_leading_nals(&v).is_ok());
    }

    #[test]
    fn missing_parameter_sets_are_refused_and_named() {
        let e = validate_leading_nals(&stream(&[32, 33, 19])).unwrap_err();
        assert!(e.contains("PPS"), "{e}");
        let e = validate_leading_nals(&stream(&[34, 19])).unwrap_err();
        assert!(e.contains("VPS") && e.contains("SPS"), "{e}");
    }

    #[test]
    fn non_idr_first_slice_is_refused() {
        // TRAIL_R (1) first: a copy that lost its head, or a cut mid-GOP.
        // Would glitch at EVERY wrap.
        let e = validate_leading_nals(&stream(&[32, 33, 34, 1])).unwrap_err();
        assert!(e.contains("not an IDR"), "{e}");
    }

    #[test]
    fn cra_open_gop_is_refused_by_name() {
        let e = validate_leading_nals(&stream(&[32, 33, 34, 21])).unwrap_err();
        assert!(e.contains("CRA"), "{e}");
    }

    #[test]
    fn parameter_sets_after_the_slice_do_not_count() {
        let e = validate_leading_nals(&stream(&[32, 33, 19, 34])).unwrap_err();
        assert!(e.contains("PPS"), "{e}");
    }

    #[test]
    fn garbage_and_degenerate_inputs_are_refused() {
        assert!(validate_leading_nals(&[]).is_err());
        let e = validate_leading_nals(&[0x47; 4096]).unwrap_err();
        assert!(e.contains("start code"), "{e}"); // no Annex-B start code at all
        // parameter sets only, no slice ever
        assert!(validate_leading_nals(&stream(&[32, 33, 34])).is_err());
        // forbidden_zero_bit set on the first NAL header
        let mut v = vec![0, 0, 0, 1, 0x80 | (32 << 1), 0x01];
        v.extend([0x2a; 8]);
        assert!(validate_leading_nals(&v).is_err());
    }
}
```

**Step 2 (1 min).** In `src/lib.rs` add `pub mod nal;` after `pub mod sidecar;`.
`cargo test --lib` — the 7 nal tests FAIL (todo!). Red.

**Step 3 (8 min).** Replace the stub body and add the private iterator:

```rust
pub fn validate_leading_nals(data: &[u8]) -> Result<(), String> {
    let mut vps = false;
    let mut sps = false;
    let mut pps = false;
    let mut found_any = false;
    let mut iter = StartCodeIter { data, i: 0 };
    while let Some((b0, _b1)) = iter.next_nal_header() {
        found_any = true;
        if b0 & 0x80 != 0 {
            return Err("corrupt NAL header (forbidden_zero_bit set)".into());
        }
        let nal_type = (b0 >> 1) & 0x3f;
        match nal_type {
            NAL_VPS => vps = true,
            NAL_SPS => sps = true,
            NAL_PPS => pps = true,
            0..=31 => {
                // First VCL NAL: the gate's decision point.
                let missing: Vec<&str> = [(!vps, "VPS"), (!sps, "SPS"), (!pps, "PPS")]
                    .iter()
                    .filter(|(m, _)| *m)
                    .map(|(_, n)| *n)
                    .collect();
                if !missing.is_empty() {
                    return Err(format!(
                        "first slice appears before parameter sets ({} missing); not a \
                         valid loop asset — re-ingest with a closed-GOP encode",
                        missing.join("/")
                    ));
                }
                return match nal_type {
                    NAL_IDR_W_RADL | NAL_IDR_N_LP => Ok(()),
                    21 => Err(
                        "leading keyframe is CRA (open GOP), not IDR; the wrap would \
                         splice mid-GOP — re-ingest with a closed-GOP encode (IDR at \
                         frame 0)"
                            .into(),
                    ),
                    t => Err(format!(
                        "first slice NAL is type {t}, not an IDR (19/20); the stream does \
                         not start on a clean keyframe — re-ingest with a closed-GOP encode"
                    )),
                };
            }
            _ => {} // other non-VCL (AUD, SEI, ...): fine before the IDR
        }
    }
    if !found_any {
        return Err(
            "no Annex-B start code found; this is not a raw HEVC elementary stream (MP4? \
             use: ffmpeg -i in.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc out.265)"
                .into(),
        );
    }
    Err("parameter sets but no slice found in the asset".into())
}

/// Finds each 00 00 01 start code (the 4-byte form contains it) and yields
/// the two NAL header bytes that follow.
struct StartCodeIter<'a> {
    data: &'a [u8],
    i: usize,
}

impl StartCodeIter<'_> {
    fn next_nal_header(&mut self) -> Option<(u8, u8)> {
        let d = self.data;
        let mut i = self.i;
        while i + 2 < d.len() {
            if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
                let h = i + 3;
                self.i = h + 1; // keep searching after this start code
                if h + 1 < d.len() {
                    return Some((d[h], d[h + 1]));
                }
                return None; // start code at EOF, no room for a header
            }
            i += 1;
        }
        self.i = i;
        None
    }
}
```

`cargo test --lib` — green.

**Step 4 (5 min).** Update `tests/cli.rs`. Replace the whole
`garbage_bytes_exit_nonzero_within_deadline` test (function and its doc comment) with:

```rust
/// Post-F4: garbage never reaches mpv — the NAL gate refuses it at startup,
/// fast, with a message naming the actual problem. (The pre-F4 version of
/// this test allowed exit 1 via mpv's demux-probe failure; the event-loop
/// hang class is covered by playback_failure_exits_nonzero_never_hangs.)
#[test]
fn garbage_bytes_refused_at_the_gate_exit_2() {
    let p = temp_path("garbage.265");
    // 64 KiB of bytes in 0x02..=0x7E: no 0x00/0x01 -> no start code anywhere.
    let bytes: Vec<u8> = (0..65536u32)
        .map(|i| ((i.wrapping_mul(2654435761) >> 24) as u8 % 0x7d) + 0x02)
        .collect();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30"); // hash MATCHES: proves the gate, not F3, refuses
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("start code"), "stderr: {}", r.stderr);
}
```

And append:

```rust
/// Wrong-but-intact: a CRA-led (open GOP) asset with a CORRECT sidecar hash.
/// F3 passes — the bytes are exactly what was ingested — and F4 must still
/// refuse, proving the hash alone is insufficient.
#[test]
fn open_gop_asset_refused_at_the_gate_exit_2() {
    let p = temp_path("opengop.265");
    let mut bytes = Vec::new();
    for t in [32u8, 33, 34, 21] {
        // VPS SPS PPS CRA
        bytes.extend([0, 0, 0, 1, t << 1, 0x01]);
        bytes.extend([0x2a; 8]);
    }
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(10),
    );
    assert_eq!(r.exit_code, Some(GATE_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("CRA"), "stderr: {}", r.stderr);
}
```

Red: rsync + Pi `cargo test` — both fail (old binary exits 1 from the mpv path).

**Step 5 (2 min).** Wire the gate. In `src/main.rs`: after the
`use dex_loop::sidecar::...;` line add:

```rust
use dex_loop::nal::validate_leading_nals;
```

and insert, directly after the closing brace of the `if let Some(s) = &sidecar { ... }`
hash-verification block and before the `eprintln!("dex-loop: {} bytes, fps ...` line:

```rust
    // F4 — validate the leading NALs. The wrap is only seamless because byte
    // 0 begins VPS/SPS/PPS + IDR; a wrong-but-intact asset (open GOP, no
    // leading IDR, not Annex-B at all) would glitch at every wrap, silently,
    // ~29k times/day. Truncation is caught by the F3 hash above; this catches
    // shape. Runs in bench mode too — the premise holds there as well.
    if let Err(e) = validate_leading_nals(leaked) {
        eprintln!("error: {path}: {e}");
        return ExitCode::from(2);
    }
```

**Step 6 (3 min).** Mac `cargo check --all-targets && cargo test --lib`; rsync + Pi
`cargo test` — all green (13 CLI tests, 3 FFI tests, all lib tests).

**Step 7 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/nal.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/tests/cli.rs
git commit -m "E: 4k-loop-probe — F4: leading-NAL gate (VPS/SPS/PPS + IDR-first, CRA refused by name)"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 8 — Build identity + startup heartbeat (F7)

**Files.** Create `build.rs`, `src/heartbeat.rs`. Modify `src/lib.rs`, `src/main.rs`,
`tests/cli.rs`.

**Interfaces.**
Produces: `dex_loop::heartbeat::format_heartbeat` (§2); env var `DEX_GIT_HASH`
(build.rs); startup line `dex-loop <version> (<hash>)`; heartbeat lines
`dex-loop: heartbeat wraps=N uptime=Ns temp=X.YC frame-drops=N vo-delayed=N`.
Consumes (new FFI, declared in main.rs's existing extern block):
`mpv_get_property_string(*mut MpvHandle, *const c_char) -> *mut c_char`,
`mpv_free(*mut c_void)`; sysfs `/sys/class/thermal/thermal_zone0/temp`; mpv properties
`frame-drop-count`, `vo-delayed-frame-count`.

**Step 1 (4 min).** Create `src/heartbeat.rs` with a stub and tests:

```rust
//! F7 — the heartbeat line: one log line every ~10 min so a weeks-later field
//! failure is diagnosable from the journal after the fact (was it degrading?
//! hot? dropping frames?). Formatting is pure and unit-tested here;
//! scheduling and the property/sysfs reads live in main.rs.

/// Render one heartbeat line. `temp_millicelsius` comes from
/// /sys/class/thermal (None off-Linux or on read failure); the two counters
/// are mpv property strings (None if the property is unavailable). Missing
/// sources degrade to "n/a" — a heartbeat must never itself be a failure.
pub fn format_heartbeat(
    wraps: u64,
    uptime_secs: u64,
    temp_millicelsius: Option<i64>,
    frame_drops: Option<&str>,
    vo_delayed: Option<&str>,
) -> String {
    let _ = (wraps, uptime_secs, temp_millicelsius, frame_drops, vo_delayed);
    todo!() // STUB — replaced in step 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_all_fields() {
        assert_eq!(
            format_heartbeat(143, 3600, Some(48_250), Some("0"), Some("2")),
            "dex-loop: heartbeat wraps=143 uptime=3600s temp=48.2C frame-drops=0 vo-delayed=2"
        );
    }

    #[test]
    fn missing_sources_degrade_to_na_not_errors() {
        assert_eq!(
            format_heartbeat(0, 0, None, None, None),
            "dex-loop: heartbeat wraps=0 uptime=0s temp=n/a frame-drops=n/a vo-delayed=n/a"
        );
    }
}
```

Add `pub mod heartbeat;` to `src/lib.rs` after `pub mod nal;`. `cargo test --lib` → the
two heartbeat tests fail. Red. Then replace the stub body:

```rust
    let temp = match temp_millicelsius {
        Some(m) => format!("{}.{}C", m / 1000, (m % 1000).abs() / 100),
        None => "n/a".to_string(),
    };
    format!(
        "dex-loop: heartbeat wraps={wraps} uptime={uptime_secs}s temp={temp} \
         frame-drops={} vo-delayed={}",
        frame_drops.unwrap_or("n/a"),
        vo_delayed.unwrap_or("n/a"),
    )
```

`cargo test --lib` → green.

**Step 2 (4 min).** Create `build.rs` in the crate root:

```rust
//! Embed the git commit into the binary so the startup line identifies the
//! exact build. std only — zero dependencies — and a missing git or a
//! non-checkout build (e.g. the rsync'd Pi mirror) degrades to "nogit"
//! rather than failing the build. Ship builds should come from a checkout.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nogit".to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=DEX_GIT_HASH={hash}{}",
        if dirty { "+dirty" } else { "" }
    );
    // Re-run when HEAD moves (crate sits 3 levels below the repo root). The
    // paths may not exist in a non-checkout build; that is fine.
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/refs");
}
```

**Step 3 (5 min).** Append the failing CLI tests to `tests/cli.rs`:

```rust
// ---- F7: build identity + heartbeat (task 8) -----------------------------

#[test]
fn startup_identifies_version_and_build() {
    // Even a refused start must identify its build — a field journal that
    // begins with an unidentifiable process is undebuggable weeks later.
    let r = run_with_deadline(&["/nonexistent/x.265"], Duration::from_secs(10));
    assert_eq!(r.exit_code, Some(GATE_EXIT));
    assert!(
        r.stderr
            .contains(&format!("dex-loop {}", env!("CARGO_PKG_VERSION"))),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn heartbeat_zero_is_emitted_at_startup() {
    // The 10-minute cadence is untestable in a test budget; heartbeat #0
    // right after loadfile proves the whole mechanism (property reads,
    // temperature, formatting) on every boot — and therefore here.
    let p = temp_path("heartbeat.265");
    let bytes = stub_annexb();
    std::fs::write(&p, &bytes).unwrap();
    write_sidecar(&p, &bytes, "30");
    let r = run_with_deadline(
        &[
            p.to_str().unwrap(),
            "--no-defaults",
            "--opt",
            "vo=null",
            "--opt",
            "vid=no",
            "--opt",
            "aid=no",
        ],
        Duration::from_secs(30),
    );
    assert_eq!(r.exit_code, Some(RUNTIME_EXIT), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("heartbeat wraps="), "stderr: {}", r.stderr);
}
```

Red: rsync + Pi `cargo test` — both fail (no version line, no heartbeat yet).

**Step 4 (10 min).** Implement in `src/main.rs`:

(a) Extend the imports. After `use std::process::ExitCode;` add:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
```

After the `use dex_loop::nal::...;` line add:

```rust
use dex_loop::heartbeat::format_heartbeat;
```

(b) Inside the existing `#[link(name = "mpv")] extern "C" { ... }` block, after the
`mpv_stream_cb_add_ro` declaration, add:

```rust
    fn mpv_get_property_string(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_free(data: *mut c_void);
```

(c) After the `const _: () = { ... assert_send::<LoopStream>(); };` block, add:

```rust
/// Completed passes over the payload — incremented by `read_fn` on the demux
/// thread each time the position wraps to 0, read by the heartbeat on the
/// event thread. This counts DEMUXER passes, which run ~1 s (readahead)
/// ahead of what is on screen. Relaxed ordering: a monotonic diagnostic
/// counter, not a synchronization point.
static WRAP_COUNT: AtomicU64 = AtomicU64::new(0);

/// Heartbeat cadence: frequent enough to bound "when did it die" to a useful
/// journal window, rare enough to cost nothing.
const HEARTBEAT_SECS: u64 = 600;
```

(d) In `read_fn`, after the line `s.pos = c.next_pos;` insert:

```rust
    if c.next_pos == 0 {
        // The copy reached the payload's end: one full pass completed.
        WRAP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
```

(e) After the `set_opt` function, add the three helpers:

```rust
/// Read an mpv property as a string, or None if unavailable. Used only by
/// the low-frequency heartbeat — never on the decode path.
fn get_prop(ctx: *mut MpvHandle, name: &str) -> Option<String> {
    let n = CString::new(name).ok()?;
    // SAFETY: ctx is a valid initialized handle; mpv returns NULL or a
    // NUL-terminated string that must be released with mpv_free.
    let p = unsafe { mpv_get_property_string(ctx, n.as_ptr()) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    // SAFETY: p came from mpv_get_property_string and is released exactly once.
    unsafe { mpv_free(p as *mut c_void) };
    Some(s)
}

/// Pi SoC temperature in millidegrees C, if the kernel exposes it. Absent on
/// non-Linux and never an error: the heartbeat degrades to n/a.
fn read_temp_millicelsius() -> Option<i64> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// One heartbeat line to stderr. Runs on the event thread, off the decode
/// path; two property reads and one sysfs read per 10 minutes is noise.
fn emit_heartbeat(ctx: *mut MpvHandle, started: Instant) {
    let drops = get_prop(ctx, "frame-drop-count");
    let delayed = get_prop(ctx, "vo-delayed-frame-count");
    eprintln!(
        "{}",
        format_heartbeat(
            WRAP_COUNT.load(Ordering::Relaxed),
            started.elapsed().as_secs(),
            read_temp_millicelsius(),
            drops.as_deref(),
            delayed.as_deref(),
        )
    );
}
```

(f) Make the very first statement of `main()` (before the `let args` line):

```rust
    // Identify the build before anything can fail: a field journal that
    // starts with an unidentifiable process is undebuggable weeks later.
    eprintln!(
        "dex-loop {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("DEX_GIT_HASH")
    );
```

(g) Replace the event loop. The existing block, beginning
`let mut exit = ExitCode::SUCCESS;` and the `loop {` with `mpv_wait_event(ctx, -1.0)`,
becomes (the long END_FILE comment block above it stays):

```rust
    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    // Heartbeat #0: proves the whole mechanism (property reads, temperature,
    // formatting) on every boot, and anchors the journal.
    emit_heartbeat(ctx, started);

    let mut exit = ExitCode::SUCCESS;
    loop {
        // Wake at least every 30 s: in the healthy steady state mpv delivers
        // NO events, which is precisely when the heartbeat must still fire.
        // A 30 s wake on the event thread costs nothing on the decode path.
        let ev = unsafe { mpv_wait_event(ctx, 30.0) };
        let id = unsafe { (*ev).event_id };

        if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_SECS {
            emit_heartbeat(ctx, started);
            last_heartbeat = Instant::now();
        }

        if id == MPV_EVENT_NONE {
            continue;
        }
        if id == MPV_EVENT_SHUTDOWN {
            break;
        }
        if id == MPV_EVENT_START_FILE {
            continue;
        }
        if id == MPV_EVENT_LOG_MESSAGE {
            // SAFETY: mpv guarantees `data` is an mpv_event_log_message for
            // this event id, with NUL-terminated strings valid until the next
            // mpv_wait_event call.
            let m = unsafe { &*((*ev).data as *const MpvEventLogMessage) };
            let pfx = unsafe { CStr::from_ptr(m.prefix) }.to_string_lossy();
            let txt = unsafe { CStr::from_ptr(m.text) }.to_string_lossy();
            eprint!("mpv/{pfx}: {txt}");
            continue;
        }
        if id == MPV_EVENT_END_FILE {
            // SAFETY: `data` is an mpv_event_end_file for this event id.
            let ef = unsafe { &*((*ev).data as *const MpvEventEndFile) };
            let why = unsafe { CStr::from_ptr(mpv_error_string(ef.error)) };
            eprintln!(
                "dex-loop: FATAL: playback ended (reason={}, error={}) -- an endless \
                 stream must never end; exiting so the supervisor restarts",
                ef.reason,
                why.to_string_lossy()
            );
            exit = ExitCode::FAILURE;
            break;
        }
    }
```

**Step 5 (3 min).** Mac `cargo check --all-targets && cargo test --lib`; rsync + Pi
`cargo test` — all green (15 CLI, 3 FFI, all lib tests). Eyeball one thing in the
`heartbeat_zero...` failure output if it flakes: heartbeat #0 must appear even though the
run ends in under a second.

**Step 6 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/build.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/heartbeat.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/lib.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/src/main.rs \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/tests/cli.rs
git commit -m "E: 4k-loop-probe — F7: version+git hash in startup line, 10-min heartbeat with boot proof"
git push origin experiment/4k-hevc-perfect-loop
```

---

### Task 9 — Docs + deploy alignment + full-matrix verification

**Files.** Modify `README.md`, `deploy/dex-loop.service`.

**Interfaces.** Documents the CLI contract produced by tasks 6–8. No code.

**Step 1 (2 min).** In `README.md`, replace the top usage block

```bash
# once, at ingest -- mpv needs a raw elementary stream, not MP4
ffmpeg -i card.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc loop.265

# then
dex-loop loop.265 --fps 30 --mode 3840x2160@30
```

with:

```bash
# once, at ingest -- mpv needs a raw elementary stream, not MP4:
ffmpeg -i card.mp4 -c:v copy -bsf:v hevc_mp4toannexb -f hevc loop.265
# and the binding sidecar (fps + hash travel WITH the asset):
printf '{"fps":"30","sha256":"%s","width":3840,"height":2160}\n' \
  "$(shasum -a 256 loop.265 | cut -d' ' -f1)" > loop.265.json   # Linux: sha256sum

# then
dex-loop loop.265 --mode 3840x2160@30
```

**Step 2 (5 min).** Replace the whole `## Options` section with:

````markdown
### Task 10 — Soak monitor run identity and schema guard (T6)

*Added after the plan was first written, from a defect observed while the plan was
being executed. Harness code, not the player: it can be done independently of
Tasks 1–9 and touches no file they touch.*

**Files:**
- Modify: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/soak-monitor.sh`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/test-soak-monitor.sh` (create)

**Interfaces:**
- Consumes: nothing from other tasks — the monitor is standalone harness code on the Mac.
- Produces: a TSV whose first column is `run_id`, and a non-zero exit (3) when the
  output file's existing header does not match the header this version writes.

**Why this task exists.** Observed 2026-08-15: the soak was restarted against the same
output file when its duration changed from 12 h to 3 h. The monitor appends, and writes
a header only when the file is empty, so run 2's rows joined run 1's and `elapsed_s`
restarts at 0 mid-file. Every row is individually honest — `iso_time` is correct — while
the series is not: anything plotting elapsed sees time run backwards, with no warning.
The same gap bit once before, when adding `held_at` took the column count from 10 to 11
and differently-shaped rows were appended to an old file in silence.

- [ ] **Step 1: Write the failing test**

Create `scripts/test-soak-monitor.sh`:

```bash
#!/usr/bin/env bash
# Self-test for soak-monitor.sh. A monitor that has only ever seen success is untested.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

fails=0
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

# 1. A fresh file gets a header whose first column is run_id.
./scripts/soak-monitor.sh --out "$t/a.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
head -1 "$t/a.tsv" | grep -q '^run_id	iso_time' \
  || { echo "FAIL: header does not start with run_id"; fails=$((fails + 1)); }

# 2. Every data row carries a non-empty run_id, and one run uses exactly one value.
ids=$(awk -F'\t' 'NR>1 {print $1}' "$t/a.tsv" | sort -u | grep -c .)
[ "$ids" = "1" ] || { echo "FAIL: expected 1 run_id in one run, got $ids"; fails=$((fails + 1)); }

# 3. A SECOND run appended to the same file uses a DIFFERENT run_id, so the merge is
#    visible rather than silent. This is the defect that prompted the task.
sleep 1
./scripts/soak-monitor.sh --out "$t/a.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
ids=$(awk -F'\t' 'NR>1 {print $1}' "$t/a.tsv" | sort -u | grep -c .)
[ "$ids" = "2" ] || { echo "FAIL: expected 2 run_ids after a restart, got $ids"; fails=$((fails + 1)); }

# 4. A file with a DIFFERENT schema is refused, not appended to.
printf 'iso_time\telapsed_s\tframes\n2026-01-01T00:00:00+00:00\t0\t1\n' > "$t/old.tsv"
./scripts/soak-monitor.sh --out "$t/old.tsv" --interval 1 --duration 1 --frames 60 >/dev/null 2>&1
rc=$?
[ "$rc" = "3" ] || { echo "FAIL: expected exit 3 on schema mismatch, got $rc"; fails=$((fails + 1)); }
[ "$(wc -l < "$t/old.tsv")" = "2" ] || { echo "FAIL: monitor appended to a mismatched file"; fails=$((fails + 1)); }

[ "$fails" = "0" ] && echo "test-soak-monitor: PASS" || echo "test-soak-monitor: $fails FAILED"
exit "$fails"
```

```bash
chmod +x scripts/test-soak-monitor.sh
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `./scripts/test-soak-monitor.sh`
Expected: FAIL on the header assertion (no `run_id` column exists yet) and on the
schema-mismatch assertion (exit is currently 0, and the row is appended).

- [ ] **Step 3: Add the run id and the header guard**

In `scripts/soak-monitor.sh`, replace the single header line:

```bash
[ -s "$OUT" ] || printf 'iso_time\telapsed_s\tframes\tnulls\twraps\theld\tmax_dwell\theld_at\tplayer\ttemp_c\tthrottled\n' >"$OUT"
```

with:

```bash
# One identity per invocation. Date alone is not enough: two runs can start in the
# same second, and then the merge this exists to expose would be invisible again.
RUN_ID="$(date +%Y%m%dT%H%M%S)-$$"

HEADER=$'run_id\tiso_time\telapsed_s\tframes\tnulls\twraps\theld\tmax_dwell\theld_at\tplayer\ttemp_c\tthrottled'

# Appending is only safe if the file already has THIS schema. The header was
# previously written only when the file was empty, so a schema change appended
# differently-shaped rows to an old file in silence -- which happened once, when
# held_at took the column count from 10 to 11.
if [ -s "$OUT" ]; then
  existing="$(head -1 "$OUT")"
  if [ "$existing" != "$HEADER" ]; then
    echo "soak-monitor: $OUT was written by a different schema; refusing to append." >&2
    echo "  expected: $HEADER" >&2
    echo "  found:    $existing" >&2
    echo "  move it aside or pass a different --out." >&2
    exit 3
  fi
else
  printf '%s\n' "$HEADER" >"$OUT"
fi
```

Then prepend `run_id` to the row that is written each sample. Replace:

```bash
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(date -Iseconds)" "$((now - start))" "$frames" "$nulls" "$wraps" \
    "$held" "$maxd" "$heldat" "$player" "$temp" "$thr" >>"$OUT"
```

with:

```bash
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$RUN_ID" "$(date -Iseconds)" "$((now - start))" "$frames" "$nulls" "$wraps" \
    "$held" "$maxd" "$heldat" "$player" "$temp" "$thr" >>"$OUT"
```

And the LINK-FAIL row, which must carry the same identity. Replace:

```bash
    printf '%s\t%s\tLINK-FAIL\n' "$(date -Iseconds)" "$((now - start))" >>"$OUT"
```

with:

```bash
    printf '%s\t%s\t%s\tLINK-FAIL\n' "$RUN_ID" "$(date -Iseconds)" "$((now - start))" >>"$OUT"
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `./scripts/test-soak-monitor.sh`
Expected: `test-soak-monitor: PASS`

Also run: `shellcheck scripts/soak-monitor.sh scripts/test-soak-monitor.sh`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/scripts/soak-monitor.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/scripts/test-soak-monitor.sh
git commit -m "E: 4k-loop-probe — soak monitor: run_id column and a header guard

Restarting the monitor against the same output file silently merged two runs:
elapsed_s restarted at 0 mid-file while every iso_time stayed correct, so the
rows were individually honest and the series was not. A run_id column makes the
merge visible instead. The header guard covers the same gap for schema changes,
which already appended 10-column rows under an 11-column header once.

Adds a self-test, because the monitor has now produced three of its own bugs and
one that has only ever seen success is untested."
```

---

## Options and the sidecar

`dex-loop <asset>` requires `<asset>.json` next to the asset — written at ingest,
binding the two facts that are undetectable when wrong: the frame rate (a raw stream
has no timestamps; a wrong rate plays slow forever with every metric nominal) and the
exact bytes (sha256 — a truncated copy glitches at every wrap).

```json
{"fps":"30","sha256":"<64 hex>","width":3840,"height":2160}
```

`fps` is a string (`"30"`, `"29.97"`, `"30000/1001"`), passed verbatim to mpv.
Required: `fps`, `sha256`. Optional: `width`, `height`, `source`, `encoder_cmd`.
Unknown keys are ignored. The parser is a strict JSON subset (flat object, strings +
unsigned integers); anything else refuses startup — fail closed.

| flag | meaning |
|---|---|
| `--fps F` | optional cross-check; must equal the sidecar fps exactly, or startup is refused |
| `--mode WxH@R` | force a DRM mode, e.g. `3840x2160@30`. Default: connector preferred. Deliberately NOT in the sidecar: mode is venue config, not asset metadata |
| `--bench-no-sidecar` | BENCH ONLY: skip the sidecar and take `--fps` as given (both flags required — the escape hatch is a deliberate two-flag act) |
| `--opt K=V` | pass any extra mpv option (repeatable) |
| `--no-defaults` | omit the built-in Pi 4 zero-copy option set |

Startup gates, in order: asset readable and non-empty → sidecar parses → fps resolved →
sha256 matches → leading NALs are VPS/SPS/PPS + IDR (open-GOP/CRA assets are refused —
the wrap premise is "IDR at frame 0"). Exit codes: **2** = refused before playback
(fix the asset/invocation; restarting cannot help), **1** = playback/runtime failure
(the supervisor restarts). Every start logs `dex-loop <version> (<git hash>)` and a
heartbeat line (`wraps=`, `temp=`, `frame-drops=`) at boot and every 10 minutes.
````

**Step 3 (3 min).** Append after the `## Building` section:

````markdown
## Testing

```bash
# Mac (no libmpv): type-check everything, run the pure-logic tests
cargo check --all-targets && cargo test --lib

# Pi (dexpi@dexpi4.local, crate mirrored at ~/bench/dex-loop): full suite
cargo test
```

The integration tests (`tests/cli.rs`, `tests/ffi_constants.rs`) link libmpv and spawn
the real binary — Pi only. They never touch the display: every playback-reaching
invocation uses `--no-defaults --opt vo=null --opt vid=no --opt aid=no`, so they are
safe to run while a soak owns the screen (`nice -n 19 cargo test` to keep builds off
the soak's CPU). See IMPLEMENTATION-PLAN.md for the full hardening rationale.
````

**Step 4 (2 min).** In `deploy/dex-loop.service`, replace:

```ini
ExecStart=/usr/local/bin/dex-loop /opt/dex/loop.265 --fps 30 --mode 3840x2160@30
```

with:

```ini
# No --fps here: the frame rate comes from /opt/dex/loop.265.json, written at
# ingest and hash-bound to the asset. Deploy BOTH files. A missing or stale
# sidecar makes the player refuse (exit 2) rather than guess a rate -- a
# wrong guess would play slow forever with every metric green.
ExecStart=/usr/local/bin/dex-loop /opt/dex/loop.265 --mode 3840x2160@30
```

**Step 5 (3 min).** Full-matrix gate:

```bash
cd /Users/mfa/CODE/dex/experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop
cargo check --all-targets && cargo test --lib
rsync -a --delete --exclude target -e "ssh -i ~/.ssh/id_ed25519" \
  ./ dexpi@dexpi4.local:~/bench/dex-loop/
ssh -i ~/.ssh/id_ed25519 dexpi@dexpi4.local \
  'cd ~/bench/dex-loop && nice -n 19 cargo test 2>&1 | tail -25'
```

Everything green on both hosts.

**Step 6 (1 min).** Commit:

```bash
cd /Users/mfa/CODE/dex
git add experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/README.md \
        experiments/2026-08-12-4k-hevc-perfect-loop/dex-loop/deploy/dex-loop.service
git commit -m "D: 4k-loop-probe — dex-loop docs: sidecar contract, exit codes, testing; unit drops --fps"
git push origin experiment/4k-hevc-perfect-loop
```

---

## 4. Explicitly NOT done here (documented as not-done)

| Item | Status | Where the design lives |
|---|---|---|
| **F1** — tier-0 health check + VO re-init | **Not implemented.** The stall watchdog (`time-pos` sampled ~10 s, in-place `loadfile` recovery, capped attempts, escalate to exit). §1.15 strengthens its case: post-gate "recognized but stalled" is invisible in-process without it. The task-8 heartbeat gives it a natural home (same thread, same cadence machinery). | PLAN.md F1 |
| **F2** — restart-burst → reboot escalation | **Not implemented.** Needs a second systemd unit (`OnFailure=` + a burst counter); deliberately not `StartLimitAction=reboot`, which conflicts with the load-bearing `StartLimitIntervalSec=0`. | PLAN.md F2 |
| **F5** — read-only rootfs + spare card | **Not implemented.** Not code. Overlay FS, swap off, flashed spare taped to the plinth. | PLAN.md F5 |
| **F6** — baked EDID | **Not implemented.** Not code; `cmdline.txt`. The capture procedure is already documented in `deploy/dex-wait-hdmi`'s header. | PLAN.md F6, deploy/dex-wait-hdmi |
| **T3 "no display present"** | **Manual only, deferred until the soak releases the display:** unplug HDMI, run with default options from SSH, expect a nonzero exit within seconds (VO init failure → END_FILE) and a journal line naming it. Cannot be automated while a soak owns the display. | this plan §1.16 |
| **T4** — clippy pedantic sweep + Miri | **Partially done:** `#![deny(unsafe_op_in_unsafe_fn)]` (main.rs) and `#![forbid(unsafe_code)]` (lib) land in task 1. The `cargo clippy -- -D warnings` sweep and a Miri run over the lib tests remain follow-ups. | PLAN.md T4 |
| **T5** — soak-monitor self-test | **Not implemented** (harness code, not crate code). | PLAN.md T5 |

The **multi-day soak on the final build** (PLAN.md phasing step 7) happens after all nine
tasks land, on the show asset, on the show hardware — soaking code that is about to
change measures the wrong artifact.
