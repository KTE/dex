# 4K HEVC Perfect Loop — Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the measurement harness that decides whether an `mpv` invocation on Pi OS Trixie loops 4K HEVC without a seam — producing a verdict backed by three controls rather than an impression.

**Architecture:** A generated test video carries its own frame index as a binary barcode burned into the top of every frame. ffmpeg crops and downscales that strip to 576 bytes per frame before Node ever sees it, so decoding is cheap at any resolution. Capture converts pixels to a stream of integers and stops; analysis takes that integer stream and produces a verdict, touching no hardware. That split makes the analyzer unit-testable against synthetic logs and lets any historical capture be replayed against revised analysis.

**Tech Stack:** Node 25 (`node --test`, zero runtime dependencies), ffmpeg 8.1 (`libx265`, `libx264`, `geq`, `drawbox`, `avfoundation`), bash + shellcheck, TypeScript as a devDependency for `tsc --noEmit` checking of JSDoc-annotated `.mjs`.

**Spec:** [../SPEC.md](../SPEC.md) · **Running record:** [../LOG-4k-hevc-perfect-loop.md](../LOG-4k-hevc-perfect-loop.md)

## Global Constraints

- **Zero runtime dependencies.** `.mjs` with `// @ts-check` and JSDoc. `typescript` is the only devDependency. The harness must run on the bench Pi, the Mac, or a borrowed laptop with nothing but Node installed.
- **Working directory** for all paths: `experiments/2026-08-12-4k-hevc-perfect-loop/` inside `~/CODE/dex`.
- **Branch:** `experiment/4k-hevc-perfect-loop`. Never merge to `main`. Push after each task.
- **Commit notation:** `E:` for harness code and scripts, `D:` for documentation (lab-notes convention).
- **Every `.sh` file passes `shellcheck` before commit.** No exceptions, no `# shellcheck disable` without a comment explaining why.
- **Barcode geometry is defined once**, in `lib/barcode.mjs`, and consumed by both the burner and the decoder. Shell scripts obtain it by calling Node, never by re-deriving it.
- **Statistical gate:** two-proportion test, `alpha = 0.01`, minimum 500 wrap transitions.
- **Test assets stay out of git.** Generated video goes in `out/`, which is gitignored. Only code, tests, and small fixtures are committed.
- **After each task, append a dated entry to the Running Log** in `LOG-4k-hevc-perfect-loop.md`. Append only — never rewrite an earlier entry; correct with a new entry.

---

### Task 1: Statistics primitive + project scaffolding

The two-proportion test is the gate the entire verdict rests on, and it is pure arithmetic with no dependencies — so it is the smallest unit that proves the test harness itself works end to end.

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/package.json`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/tsconfig.json`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/.gitignore`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/lib/stats.mjs`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/stats.test.mjs`

**Interfaces:**
- Consumes: nothing.
- Produces: `erf(x: number): number`, `normalCdf(z: number): number`, `twoProportionTest({x1: number, n1: number, x2: number, n2: number}): {p1: number, p2: number, z: number, p: number}`.

- [ ] **Step 1: Create the scaffolding files**

`package.json`:

```json
{
  "name": "dex-4k-loop-probe",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Measurement harness for the dex 4K HEVC seamless-loop experiment",
  "scripts": {
    "test": "node --test test/",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "typescript": "^5.7.0"
  }
}
```

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2023",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "allowJs": true,
    "checkJs": true,
    "noEmit": true,
    "strict": true,
    "types": ["node"]
  },
  "include": ["lib/**/*.mjs", "bin/**/*.mjs", "test/**/*.mjs"]
}
```

`.gitignore`:

```
node_modules/
out/
*.log
```

- [ ] **Step 2: Write the failing test**

`test/stats.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { erf, normalCdf, twoProportionTest } from '../lib/stats.mjs';

test('erf(0) is 0 and erf saturates to 1', () => {
  assert.equal(erf(0), 0);
  assert.ok(Math.abs(erf(3) - 0.9999779) < 1e-5);
  assert.ok(Math.abs(erf(-3) + 0.9999779) < 1e-5);
});

test('normalCdf matches known quantiles', () => {
  assert.ok(Math.abs(normalCdf(0) - 0.5) < 1e-9);
  assert.ok(Math.abs(normalCdf(1.959964) - 0.975) < 1e-4);
  assert.ok(Math.abs(normalCdf(-2.575829) - 0.005) < 1e-4);
});

test('identical proportions give p near 1', () => {
  const r = twoProportionTest({ x1: 50, n1: 1000, x2: 50, n2: 1000 });
  assert.equal(r.z, 0);
  assert.ok(r.p > 0.99);
});

test('a defect at every wrap is detected', () => {
  // 500 wraps, every one anomalous; mid-loop clean apart from noise.
  const r = twoProportionTest({ x1: 500, n1: 500, x2: 10, n2: 14500 });
  assert.ok(r.p1 > r.p2);
  assert.ok(r.p < 1e-6, `expected tiny p, got ${r.p}`);
});

test('zero anomalies on both sides is not a difference', () => {
  const r = twoProportionTest({ x1: 0, n1: 500, x2: 0, n2: 14500 });
  assert.equal(r.z, 0);
  assert.equal(r.p, 1);
});

test('empty sample is rejected rather than silently returning NaN', () => {
  assert.throws(() => twoProportionTest({ x1: 0, n1: 0, x2: 1, n2: 10 }), /n1 and n2 must be positive/);
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd experiments/2026-08-12-4k-hevc-perfect-loop && node --test test/stats.test.mjs`
Expected: FAIL — `Cannot find module '../lib/stats.mjs'`

- [ ] **Step 4: Write the implementation**

`lib/stats.mjs`:

```js
// @ts-check

/**
 * Error function, Abramowitz & Stegun 7.1.26.
 * Max absolute error 1.5e-7 — orders of magnitude tighter than an alpha=0.01 gate needs.
 * @param {number} x
 * @returns {number}
 */
export function erf(x) {
  const sign = x < 0 ? -1 : 1;
  const ax = Math.abs(x);
  const t = 1 / (1 + 0.3275911 * ax);
  const poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
  return sign * (1 - poly * Math.exp(-ax * ax));
}

/**
 * Standard normal cumulative distribution function.
 * @param {number} z
 * @returns {number}
 */
export function normalCdf(z) {
  return 0.5 * (1 + erf(z / Math.SQRT2));
}

/**
 * Two-proportion z-test, two-sided.
 *
 * Group 1 is the wrap-transition sample, group 2 the mid-loop sample. The null
 * hypothesis is that both come from the same underlying anomaly rate — i.e. that
 * everything observed at the wrap is explained by capture noise.
 *
 * @param {{x1: number, n1: number, x2: number, n2: number}} input
 * @returns {{p1: number, p2: number, z: number, p: number}}
 */
export function twoProportionTest({ x1, n1, x2, n2 }) {
  if (n1 <= 0 || n2 <= 0) throw new Error('n1 and n2 must be positive');
  const p1 = x1 / n1;
  const p2 = x2 / n2;
  const pooled = (x1 + x2) / (n1 + n2);
  const se = Math.sqrt(pooled * (1 - pooled) * (1 / n1 + 1 / n2));
  // Degenerate case: no anomalies anywhere (or all anomalies everywhere) means
  // there is no variance to test against. That is "no detectable difference",
  // not an error — and emphatically not a pass-by-division-by-zero.
  if (se === 0) return { p1, p2, z: 0, p: 1 };
  const z = (p1 - p2) / se;
  const p = 2 * (1 - normalCdf(Math.abs(z)));
  return { p1, p2, z, p };
}
```

- [ ] **Step 5: Run the tests and the typechecker**

Run: `pnpm install && node --test test/stats.test.mjs && pnpm typecheck`
Expected: all tests PASS, `tsc --noEmit` reports no errors.

- [ ] **Step 6: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/package.json \
        experiments/2026-08-12-4k-hevc-perfect-loop/tsconfig.json \
        experiments/2026-08-12-4k-hevc-perfect-loop/.gitignore \
        experiments/2026-08-12-4k-hevc-perfect-loop/lib/stats.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/stats.test.mjs
git commit -m "E: 4k-loop-probe — two-proportion test + project scaffolding"
git push
```

---

### Task 2: Barcode codec — burn and decode, round-trip tested

The burner and decoder are a matched pair; neither is meaningful without the other, and a reviewer could not sensibly accept one and reject the other. They ship together, validated by a round-trip that needs no hardware.

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/lib/barcode.mjs`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/bin/barcode-filter.mjs`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/add-barcode.sh`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/barcode.test.mjs`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `CELLS: 18`, `DATA_BITS: 16`, `DECODE_W: 72`, `DECODE_H: 8`, `FRAME_BYTES: 576`, `barHeight(frameHeight: number): number`, `burnFilter(frameHeight: number): string`, `decodeFilter(frameHeight: number): string`, `decodeFrame(gray: Uint8Array): number | null`.

**Barcode format** (defined here once, referenced everywhere):

A strip across the top of the frame, `round(height / 24)` pixels tall (minimum 8), divided into 18 equal-width cells:

| Cell | Content |
|---|---|
| 0 | Sync **white** — supplies the white reference level |
| 1 | Sync **black** — supplies the black reference level |
| 2 + k | Data bit *k* of the frame index, **LSB at cell 2** (k = 0..15) |

16 data bits = 65,536 frames, about 36 minutes at 30 fps. Decoding thresholds at the midpoint of the two sync cells, so it survives any brightness or contrast shift the capture path applies. If the sync cells are less than 40 levels apart the frame is unreadable — a black frame, a torn frame, or a mistimed capture — and decodes to `null` rather than to a wrong number.

- [ ] **Step 1: Write the failing test**

`test/barcode.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  CELLS, DATA_BITS, DECODE_W, DECODE_H, FRAME_BYTES,
  barHeight, burnFilter, decodeFilter, decodeFrame,
} from '../lib/barcode.mjs';

/** Build a synthetic decode-sized gray frame encoding `value`. */
function synthFrame(value, { white = 235, black = 16 } = {}) {
  const f = new Uint8Array(FRAME_BYTES).fill(black);
  const setCell = (i, level) => {
    for (let y = 0; y < DECODE_H; y++) {
      for (let x = i * 4; x < i * 4 + 4; x++) f[y * DECODE_W + x] = level;
    }
  };
  setCell(0, white);
  setCell(1, black);
  for (let k = 0; k < DATA_BITS; k++) setCell(2 + k, (value >> k) & 1 ? white : black);
  return f;
}

test('geometry constants are self-consistent', () => {
  assert.equal(CELLS, 18);
  assert.equal(CELLS, 2 + DATA_BITS);
  assert.equal(DECODE_W, CELLS * 4);
  assert.equal(FRAME_BYTES, DECODE_W * DECODE_H);
  assert.equal(barHeight(2160), 90);
  assert.equal(barHeight(96), 8, 'clamps to a floor so tiny test frames stay decodable');
});

test('decodeFrame round-trips synthetic values', () => {
  for (const v of [0, 1, 2, 255, 256, 4095, 65535]) {
    assert.equal(decodeFrame(synthFrame(v)), v, `failed for ${v}`);
  }
});

test('decodeFrame returns null when the sync cells are indistinguishable', () => {
  assert.equal(decodeFrame(new Uint8Array(FRAME_BYTES).fill(0)), null);
  assert.equal(decodeFrame(synthFrame(42, { white: 30, black: 16 })), null);
});

test('decodeFrame survives a shifted brightness range', () => {
  // Capture paths clamp and shift levels; thresholding off the sync cells must absorb it.
  assert.equal(decodeFrame(synthFrame(1234, { white: 180, black: 60 })), 1234);
});

test('burn then decode round-trips through real ffmpeg', () => {
  const dir = mkdtempSync(join(tmpdir(), 'barcode-'));
  try {
    const src = join(dir, 'src.mkv');
    const burned = join(dir, 'burned.mkv');
    const N = 40, W = 320, H = 192;
    // A plain gray source; only the barcode strip matters here.
    execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi',
      '-i', `color=c=gray:s=${W}x${H}:r=30:d=${N / 30}`, '-c:v', 'ffv1', '-y', src]);
    execFileSync('bash', ['scripts/add-barcode.sh', '--input', src, '--output', burned]);

    const raw = execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', burned,
      '-vf', decodeFilter(H), '-f', 'rawvideo', '-pix_fmt', 'gray', '-'],
      { maxBuffer: 64 * 1024 * 1024 });

    const frames = Math.floor(raw.length / FRAME_BYTES);
    assert.ok(frames >= N, `expected >= ${N} frames, got ${frames}`);
    const decoded = [];
    for (let i = 0; i < N; i++) {
      decoded.push(decodeFrame(new Uint8Array(raw.buffer, raw.byteOffset + i * FRAME_BYTES, FRAME_BYTES)));
    }
    assert.deepEqual(decoded, Array.from({ length: N }, (_, i) => i));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('burnFilter and decodeFilter agree on strip height', () => {
  assert.ok(burnFilter(1080).includes(`h=${barHeight(1080)}`));
  assert.ok(decodeFilter(1080).includes(`${barHeight(1080)}`));
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/barcode.test.mjs`
Expected: FAIL — `Cannot find module '../lib/barcode.mjs'`

- [ ] **Step 3: Write the codec library**

`lib/barcode.mjs`:

```js
// @ts-check

/** Total cells across the strip: 2 sync + 16 data. */
export const CELLS = 18;
/** Frame-index width in bits. 65,536 frames ~= 36 min at 30 fps. */
export const DATA_BITS = 16;
/** Pixels per cell after the decode downscale. */
export const CELL_PX = 4;
/** Decode-stage frame width in pixels. */
export const DECODE_W = CELLS * CELL_PX;
/** Decode-stage frame height in pixels. */
export const DECODE_H = 8;
/** Bytes per decode-stage gray frame. Independent of source resolution. */
export const FRAME_BYTES = DECODE_W * DECODE_H;
/** Minimum separation between the sync levels for a frame to be considered readable. */
export const SYNC_MIN_DELTA = 40;

/**
 * Height of the barcode strip for a given frame height.
 * Floored at 8 px so tiny test frames stay decodable after the area downscale.
 * @param {number} frameHeight
 * @returns {number}
 */
export function barHeight(frameHeight) {
  return Math.max(8, Math.round(frameHeight / 24));
}

/**
 * ffmpeg filter chain that burns the frame index into the top strip.
 *
 * Cell 0 is white and cell 1 black — these are the sync/reference cells. Data
 * cell 2+k is lit when bit k of the frame counter `n` is set; the expression
 * `bitand(floor(n/2^k),1)` is evaluated per frame by drawbox's `enable`.
 *
 * @param {number} frameHeight
 * @returns {string}
 */
export function burnFilter(frameHeight) {
  const h = barHeight(frameHeight);
  const parts = [
    `drawbox=x=0:y=0:w=iw:h=${h}:color=black@1:t=fill`,
    `drawbox=x=0:y=0:w=iw/${CELLS}:h=${h}:color=white@1:t=fill`,
  ];
  for (let k = 0; k < DATA_BITS; k++) {
    const cell = 2 + k;
    const pow = 2 ** k;
    parts.push(
      `drawbox=x=${cell}*iw/${CELLS}:y=0:w=iw/${CELLS}:h=${h}:color=white@1:t=fill:` +
      `enable='eq(bitand(floor(n/${pow})\\,1)\\,1)'`
    );
  }
  return parts.join(',');
}

/**
 * ffmpeg filter chain that reduces each frame to just the barcode strip,
 * downscaled so one decode frame is FRAME_BYTES regardless of source resolution.
 *
 * `flags=area` averages each cell, which raises noise immunity rather than
 * lowering it — the cells are large uniform blocks.
 *
 * @param {number} frameHeight
 * @returns {string}
 */
export function decodeFilter(frameHeight) {
  return `crop=iw:${barHeight(frameHeight)}:0:0,scale=${DECODE_W}:${DECODE_H}:flags=area,format=gray`;
}

/**
 * Decode one downscaled gray frame to its embedded index.
 * Returns null when the frame is unreadable — black, torn, or mistimed — so an
 * unreadable frame can never masquerade as a valid index.
 *
 * @param {Uint8Array} gray - exactly FRAME_BYTES of gray8, DECODE_W x DECODE_H
 * @returns {number | null}
 */
export function decodeFrame(gray) {
  if (gray.length !== FRAME_BYTES) throw new Error(`expected ${FRAME_BYTES} bytes, got ${gray.length}`);
  const row = Math.floor(DECODE_H / 2);
  const sample = (/** @type {number} */ cell) =>
    gray[row * DECODE_W + cell * CELL_PX + Math.floor(CELL_PX / 2)];
  const white = sample(0);
  const black = sample(1);
  if (white - black < SYNC_MIN_DELTA) return null;
  const threshold = (white + black) / 2;
  let value = 0;
  for (let k = 0; k < DATA_BITS; k++) {
    if (sample(2 + k) > threshold) value |= 1 << k;
  }
  return value;
}
```

- [ ] **Step 4: Write the filter-printing CLI**

This exists so shell scripts obtain the filter from the single definition in `lib/barcode.mjs` instead of re-deriving the geometry — the burner and decoder drifting apart would be a silent, hard-to-diagnose failure.

`bin/barcode-filter.mjs`:

```js
#!/usr/bin/env node
// @ts-check
import { burnFilter, decodeFilter, barHeight } from '../lib/barcode.mjs';

const [mode, heightArg] = process.argv.slice(2);
const height = Number(heightArg);
if (!Number.isFinite(height) || height <= 0) {
  console.error('usage: barcode-filter.mjs <burn|decode|height> <frame-height>');
  process.exit(2);
}
if (mode === 'burn') process.stdout.write(burnFilter(height));
else if (mode === 'decode') process.stdout.write(decodeFilter(height));
else if (mode === 'height') process.stdout.write(String(barHeight(height)));
else {
  console.error(`unknown mode: ${mode}`);
  process.exit(2);
}
```

- [ ] **Step 5: Write the burn script**

`scripts/add-barcode.sh`:

```bash
#!/usr/bin/env bash
# Burn the binary frame-index barcode onto a lossless test card, losslessly.
# Geometry comes from lib/barcode.mjs so the burner and decoder cannot drift apart.
set -euo pipefail

usage() { echo "usage: $0 --input <lossless-test-card> --output <out.mkv>" >&2; exit 2; }

INPUT=""; OUTPUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --input)  INPUT="$2";  shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$INPUT" ] && [ -n "$OUTPUT" ] || usage

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

HEIGHT="$(ffprobe -v error -select_streams v:0 -show_entries stream=height \
  -of csv=p=0 "$INPUT")"
FILTER="$(node "$HERE/../bin/barcode-filter.mjs" burn "$HEIGHT")"

# ffv1 keeps the test card lossless: the barcode must survive to the encode stage
# unaltered, and any generational loss here would be indistinguishable from a
# decoder artifact later.
ffmpeg -hide_banner -loglevel error -i "$INPUT" \
  -vf "$FILTER" -c:v ffv1 -level 3 -an -y "$OUTPUT"

echo "burned barcode (strip ${HEIGHT}px source -> $(node "$HERE/../bin/barcode-filter.mjs" height "$HEIGHT")px) -> $OUTPUT" >&2
```

- [ ] **Step 6: Run shellcheck, tests, and typecheck**

Run:
```bash
chmod +x scripts/add-barcode.sh bin/barcode-filter.mjs
shellcheck scripts/add-barcode.sh
node --test test/barcode.test.mjs
pnpm typecheck
```
Expected: shellcheck silent, all tests PASS, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/lib/barcode.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/bin/barcode-filter.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/scripts/add-barcode.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/barcode.test.mjs
git commit -m "E: 4k-loop-probe — barcode codec, ffmpeg round-trip tested"
git push
```

---

### Task 3: Procedural test-card generator with an exact matched-wrap proof

The test card must loop *perfectly by construction*, otherwise the experiment measures the asset rather than the player. A rotation of exactly one turn over the clip gives an exact, bit-checkable guarantee.

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/make-test-card.sh`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/make-test-card.test.mjs`

**Interfaces:**
- Consumes: nothing.
- Produces: `scripts/make-test-card.sh --width W --height H --fps F --frames N [--period P] --output out.mkv`, writing a lossless ffv1 file of exactly N frames. `--period` defaults to `--frames` and controls the rotation period; passing `--frames P+1 --period P` renders one extra frame for wrap verification.

**Why `--period` is separate from `--frames`:** rendering one frame *past* the loop point lets the test assert that frame P is bit-identical to frame 0. That is a direct proof of matched wrap rather than a visual impression, and it is only possible if the motion period can be decoupled from the render length.

- [ ] **Step 1: Write the failing test**

`test/make-test-card.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const PERIOD = 30;

function withTmp(fn) {
  const dir = mkdtempSync(join(tmpdir(), 'testcard-'));
  try { return fn(dir); } finally { rmSync(dir, { recursive: true, force: true }); }
}

/** Extract a single frame as raw gray bytes. */
function frameBytes(file, index) {
  return execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', file,
    '-vf', `select=eq(n\\,${index})`, '-vsync', '0', '-frames:v', '1',
    '-f', 'rawvideo', '-pix_fmt', 'gray', '-'], { maxBuffer: 64 * 1024 * 1024 });
}

test('frame P is bit-identical to frame 0 — the wrap is matched by construction', () => {
  withTmp((dir) => {
    const out = join(dir, 'm.mkv');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', String(PERIOD + 1), '--period', String(PERIOD), '--output', out]);
    const first = frameBytes(out, 0);
    const wrap = frameBytes(out, PERIOD);
    assert.equal(Buffer.compare(first, wrap), 0, 'frame P must equal frame 0 exactly');
  });
});

test('consecutive frames differ — the pattern actually moves', () => {
  withTmp((dir) => {
    const out = join(dir, 'm.mkv');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', String(PERIOD), '--output', out]);
    assert.notEqual(Buffer.compare(frameBytes(out, 0), frameBytes(out, 1)), 0);
  });
});

test('renders exactly the requested frame count at the requested size', () => {
  withTmp((dir) => {
    const out = join(dir, 'm.mkv');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '256', '--height', '144',
      '--fps', '30', '--frames', '15', '--output', out]);
    const probe = JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0',
      '-count_frames', '-show_entries', 'stream=nb_read_frames,width,height',
      '-of', 'json', out]).toString());
    const s = probe.streams[0];
    assert.equal(Number(s.nb_read_frames), 15);
    assert.equal(Number(s.width), 256);
    assert.equal(Number(s.height), 144);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/make-test-card.test.mjs`
Expected: FAIL — `scripts/make-test-card.sh: No such file or directory`

- [ ] **Step 3: Write the generator**

`scripts/make-test-card.sh`:

```bash
#!/usr/bin/env bash
# Generate a lossless procedural test card whose motion loops exactly.
#
# The pattern is a pair of superimposed plane waves rotated by angle
# a = 2*PI*n/PERIOD. At n = PERIOD the angle is exactly 2*PI, so the frame is
# identical to n = 0 — the wrap is matched by construction, not by eye.
#
# Two spatial frequencies are superimposed on purpose: the low one gives smooth
# visible motion, the high one gives the encoder hard-to-compress detail so the
# decoder is honestly loaded at the target bitrate. A smooth gradient would
# compress to nothing and test the decoder at a bitrate no real artwork produces.
set -euo pipefail

usage() {
  echo "usage: $0 --width W --height H --fps F --frames N [--period P] --output out.mkv" >&2
  exit 2
}

WIDTH=""; HEIGHT=""; FPS=""; FRAMES=""; PERIOD=""; OUTPUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --width)  WIDTH="$2";  shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --fps)    FPS="$2";    shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --period) PERIOD="$2"; shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$WIDTH" ] && [ -n "$HEIGHT" ] && [ -n "$FPS" ] && [ -n "$FRAMES" ] && [ -n "$OUTPUT" ] || usage
PERIOD="${PERIOD:-$FRAMES}"

CX=$(( WIDTH / 2 ))
CY=$(( HEIGHT / 2 ))

# Rotated coordinate: u = (X-CX)*cos(a) + (Y-CY)*sin(a), a = 2*PI*N/PERIOD
ROT="((X-${CX})*cos(2*PI*N/${PERIOD})+(Y-${CY})*sin(2*PI*N/${PERIOD}))"
LUM="128+70*sin(0.06*${ROT})+50*sin(0.47*${ROT})"

ffmpeg -hide_banner -loglevel error \
  -f lavfi -i "nullsrc=s=${WIDTH}x${HEIGHT}:r=${FPS}" \
  -frames:v "$FRAMES" \
  -vf "geq=lum='${LUM}':cb=128:cr=128,format=yuv420p" \
  -c:v ffv1 -level 3 -an -y "$OUTPUT"

echo "test card: ${WIDTH}x${HEIGHT}@${FPS} ${FRAMES} frames (period ${PERIOD}) -> $OUTPUT" >&2
```

- [ ] **Step 4: Run shellcheck and the tests**

Run:
```bash
chmod +x scripts/make-test-card.sh
shellcheck scripts/make-test-card.sh
node --test test/make-test-card.test.mjs
```
Expected: shellcheck silent, all three tests PASS.

If the bit-identical test fails, the cause is almost always `geq` being evaluated at a shifted `N`. Verify by rendering 2 frames with `--period 1`: every frame must then be identical.

- [ ] **Step 5: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/scripts/make-test-card.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/make-test-card.test.mjs
git commit -m "E: 4k-loop-probe — procedural test card with bit-exact matched wrap"
git push
```

---

### Task 4: Encode variants and player sidecars

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/encode-variants.sh`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/encode-variants.test.mjs`

**Interfaces:**
- Consumes: a barcoded lossless test card from Tasks 2 and 3.
- Produces: `scripts/encode-variants.sh --input <barcoded.mkv> --outdir <dir> --name <base> --fps F [--bitrate 40M] [--h264]`, writing `<base>.mp4` (HEVC), `<base>.json` (pivid timeline), `<base>.html` (Cog), and with `--h264` also `<base>.h264` (Annex-B elementary stream for `hello_video`).

**Sidecar formats** follow the convention already established in `packages/example-content/export/`.

- [ ] **Step 1: Write the failing test**

`test/encode-variants.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

function ffprobeJson(file, extra = []) {
  return JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-of', 'json', ...extra, file]).toString());
}

function buildFixture(dir) {
  const testCard = join(dir, 'card.mkv');
  const burned = join(dir, 'burned.mkv');
  execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
    '--fps', '30', '--frames', '60', '--output', testCard]);
  execFileSync('bash', ['scripts/add-barcode.sh', '--input', testCard, '--output', burned]);
  return burned;
}

test('produces HEVC with a closed GOP, an IDR at frame 0, and no audio', () => {
  const dir = mkdtempSync(join(tmpdir(), 'enc-'));
  try {
    const burned = buildFixture(dir);
    execFileSync('bash', ['scripts/encode-variants.sh', '--input', burned,
      '--outdir', dir, '--name', 'v', '--fps', '30', '--bitrate', '2M']);

    const mp4 = join(dir, 'v.mp4');
    assert.ok(existsSync(mp4));

    const streams = ffprobeJson(mp4, ['-show_streams']).streams;
    const video = streams.find((s) => s.codec_type === 'video');
    assert.equal(video.codec_name, 'hevc');
    assert.equal(streams.some((s) => s.codec_type === 'audio'), false, 'must be silent');

    const frames = ffprobeJson(mp4, ['-select_streams', 'v:0', '-show_frames',
      '-show_entries', 'frame=key_frame']).frames;
    assert.equal(Number(frames[0].key_frame), 1, 'frame 0 must be a keyframe');
    const keyframes = frames.filter((f) => Number(f.key_frame) === 1).length;
    assert.equal(keyframes, 2, 'keyint=fps over 60 frames at 30fps means 2 keyframes');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('writes pivid and cog sidecars matching the example-content convention', () => {
  const dir = mkdtempSync(join(tmpdir(), 'enc-'));
  try {
    const burned = buildFixture(dir);
    execFileSync('bash', ['scripts/encode-variants.sh', '--input', burned,
      '--outdir', dir, '--name', 'v', '--fps', '30', '--bitrate', '2M']);

    const pivid = JSON.parse(readFileSync(join(dir, 'v.json'), 'utf8'));
    const screen = pivid.screens['HDMI-1'];
    assert.deepEqual(screen.mode, [320, 192, 30]);
    assert.equal(screen.layers[0].media, 'v.mp4');
    assert.equal(screen.layers[0].play.repeat, true);

    const html = readFileSync(join(dir, 'v.html'), 'utf8');
    assert.match(html, /<video[^>]*\bloop\b/);
    assert.match(html, /v\.mp4/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('--h264 emits a raw Annex-B elementary stream for hello_video', () => {
  const dir = mkdtempSync(join(tmpdir(), 'enc-'));
  try {
    const burned = buildFixture(dir);
    execFileSync('bash', ['scripts/encode-variants.sh', '--input', burned,
      '--outdir', dir, '--name', 'v', '--fps', '30', '--bitrate', '2M', '--h264']);
    const raw = join(dir, 'v.h264');
    assert.ok(existsSync(raw));
    const head = readFileSync(raw).subarray(0, 4);
    assert.deepEqual([...head], [0, 0, 0, 1], 'must start with an Annex-B start code');
    assert.equal(ffprobeJson(raw, ['-show_streams']).streams[0].codec_name, 'h264');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/encode-variants.test.mjs`
Expected: FAIL — `scripts/encode-variants.sh: No such file or directory`

- [ ] **Step 3: Write the encoder**

`scripts/encode-variants.sh`:

```bash
#!/usr/bin/env bash
# Encode a barcoded lossless test card into the player-facing variants plus sidecars.
#
# GOP settings are not incidental: keyint=min-keyint=fps with scenecut=0 and
# open-gop=0 gives a closed GOP with an IDR every second and one at frame 0.
# An open GOP would make frame 0 depend on frames that no longer exist at the
# wrap, which is a way to manufacture the very seam we are trying to measure.
set -euo pipefail

usage() {
  echo "usage: $0 --input <barcoded.mkv> --outdir <dir> --name <base> --fps F [--bitrate 40M] [--h264]" >&2
  exit 2
}

INPUT=""; OUTDIR=""; NAME=""; FPS=""; BITRATE="40M"; WANT_H264=0
while [ $# -gt 0 ]; do
  case "$1" in
    --input)   INPUT="$2";   shift 2 ;;
    --outdir)  OUTDIR="$2";  shift 2 ;;
    --name)    NAME="$2";    shift 2 ;;
    --fps)     FPS="$2";     shift 2 ;;
    --bitrate) BITRATE="$2"; shift 2 ;;
    --h264)    WANT_H264=1;  shift ;;
    *) usage ;;
  esac
done
[ -n "$INPUT" ] && [ -n "$OUTDIR" ] && [ -n "$NAME" ] && [ -n "$FPS" ] || usage
mkdir -p "$OUTDIR"

read -r WIDTH HEIGHT <<< "$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=p=0:s=' ' "$INPUT")"

X265_PARAMS="keyint=${FPS}:min-keyint=${FPS}:scenecut=0:open-gop=0:repeat-headers=1"
ffmpeg -hide_banner -loglevel error -i "$INPUT" \
  -c:v libx265 -x265-params "$X265_PARAMS" \
  -b:v "$BITRATE" -pix_fmt yuv420p -tag:v hvc1 -an -y "$OUTDIR/$NAME.mp4"

if [ "$WANT_H264" -eq 1 ]; then
  # hello_video plays only raw H.264 elementary streams, so this is the
  # positive-control asset: the same content the proven loop can play.
  ffmpeg -hide_banner -loglevel error -i "$INPUT" \
    -c:v libx264 -x264-params "keyint=${FPS}:min-keyint=${FPS}:scenecut=0:open-gop=0" \
    -b:v "$BITRATE" -pix_fmt yuv420p -an -f h264 -y "$OUTDIR/$NAME.h264"
fi

cat > "$OUTDIR/$NAME.json" <<EOF
{
  "screens": {
    "HDMI-1": {
      "mode": [${WIDTH}, ${HEIGHT}, ${FPS}],
      "update_hz": ${FPS},
      "layers": [{
          "media": "${NAME}.mp4",
          "play": {"t": [0, 0], "rate": 1, "repeat": true},
          "buffer": 2
      }]
    }
  }
}
EOF

cat > "$OUTDIR/$NAME.html" <<EOF
<!-- for usage in a fullscreen browser like cog -->
<video autoplay muted loop src="${NAME}.mp4"></video>
EOF

echo "encoded ${WIDTH}x${HEIGHT}@${FPS} -> $OUTDIR/$NAME.{mp4,json,html}" >&2
```

- [ ] **Step 4: Run shellcheck and the tests**

Run:
```bash
chmod +x scripts/encode-variants.sh
shellcheck scripts/encode-variants.sh
node --test test/encode-variants.test.mjs
```
Expected: shellcheck silent, all three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/scripts/encode-variants.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/encode-variants.test.mjs
git commit -m "E: 4k-loop-probe — encode variants + pivid/cog/hello_video sidecars"
git push
```

---

### Task 5: The analyzer — transition classification, controls, verdict

This is where the experiment's conclusion is actually computed, and it is pure: a list of integers in, a verdict out. Every control from SPEC.md §4.3 is exercised here without any hardware.

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/lib/analyze.mjs`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/bin/analyze.mjs`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/analyze.test.mjs`

**Interfaces:**
- Consumes: `twoProportionTest` from `lib/stats.mjs` (Task 1).
- Produces: `analyze(indices: (number|null)[], opts?: {loopLength?: number, alpha?: number, minWraps?: number}): AnalyzeResult`, where `AnalyzeResult` has `loopLength`, `wrapTransitions`, `wrapAnomalies`, `midTransitions`, `midAnomalies`, `wrapRate`, `midRate`, `z`, `p`, `decodeFailures`, `verdict: 'PASS'|'FAIL'|'VOID'|'INSUFFICIENT'`, `reason: string`.

**Transition classification rules** — walk consecutive pairs `(prev, cur)`, tracking `lastGood` (the most recent non-null index):

| Condition | Bucket | Anomalous? |
|---|---|---|
| `prev` or `cur` is `null` | `wrap` if `lastGood === N-1`, else `mid` | yes; also `decodeFailures++` |
| `cur === prev + 1` | `mid` | no |
| `prev === N-1 && cur === 0` | `wrap` | no — this is a clean wrap |
| `cur < prev` (any other reset) | `wrap` | yes |
| `prev === N-1` (held or skipped at the boundary) | `wrap` | yes |
| anything else (repeat or skip mid-loop) | `mid` | yes |

**Verdict rules:**

| Condition | Verdict |
|---|---|
| `wrapTransitions < minWraps` | `INSUFFICIENT` |
| `p >= alpha` | `PASS` |
| `p < alpha && wrapRate > midRate` | `FAIL` |
| `p < alpha && wrapRate < midRate` | `VOID` |

`VOID` matters: a wrap anomaly rate *below* the mid-loop baseline is not a better result, it means wraps are being misclassified — most likely `loopLength` is wrong — and every number in that run is untrustworthy.

- [ ] **Step 1: Write the failing test**

`test/analyze.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { analyze } from '../lib/analyze.mjs';

const N = 30;

/** Perfect capture: `wraps` clean loops of length N. */
function perfect(wraps) {
  const out = [];
  for (let w = 0; w < wraps; w++) for (let i = 0; i < N; i++) out.push(i);
  return out;
}

/** Deterministic LCG so the noise test is reproducible. */
function lcg(seed) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

test('a perfect capture passes', () => {
  const r = analyze(perfect(600), { loopLength: N });
  assert.equal(r.verdict, 'PASS');
  assert.equal(r.wrapAnomalies, 0);
  assert.equal(r.midAnomalies, 0);
  assert.ok(r.wrapTransitions >= 500);
});

test('a held frame at every wrap fails', () => {
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) out.push(i);
    out.push(N - 1); // last frame held one extra capture frame
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
  assert.ok(r.wrapRate > r.midRate);
});

test('a dropped frame at every wrap fails', () => {
  const out = [];
  for (let w = 0; w < 600; w++) for (let i = 0; i < N - 1; i++) out.push(i); // N-1 never shown
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
});

test('a black frame at every wrap fails and is counted as a decode failure', () => {
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) out.push(i);
    out.push(null);
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
  assert.equal(r.decodeFailures, 1200, 'each null spans two transitions');
});

test('CONTROL: uniform capture noise passes — it is the noise floor, not a seam', () => {
  // 1% of frames duplicated at uniformly random positions. This is the control
  // that makes the whole method valid: if this failed, every run would fail.
  const rnd = lcg(12345);
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) {
      out.push(i);
      if (rnd() < 0.01) out.push(i);
    }
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'PASS', `noise must not read as a seam (p=${r.p})`);
});

test('anomalies concentrated mid-loop void the run rather than passing it', () => {
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) {
      out.push(i);
      if (i === 10) out.push(i); // deterministic mid-loop stutter, never at the wrap
    }
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'VOID');
});

test('too few wraps is INSUFFICIENT, not PASS', () => {
  const r = analyze(perfect(100), { loopLength: N });
  assert.equal(r.verdict, 'INSUFFICIENT');
});

test('a single defect in 600 wraps is below the noise floor and passes', () => {
  const out = perfect(600);
  out.splice(N * 300, 0, N - 1); // one held frame, once
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'PASS');
});

test('loopLength is inferred from the data when not supplied', () => {
  const r = analyze(perfect(600));
  assert.equal(r.loopLength, N);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/analyze.test.mjs`
Expected: FAIL — `Cannot find module '../lib/analyze.mjs'`

- [ ] **Step 3: Write the analyzer library**

`lib/analyze.mjs`:

```js
// @ts-check
import { twoProportionTest } from './stats.mjs';

/**
 * @typedef {Object} AnalyzeResult
 * @property {number} loopLength
 * @property {number} wrapTransitions
 * @property {number} wrapAnomalies
 * @property {number} midTransitions
 * @property {number} midAnomalies
 * @property {number} wrapRate
 * @property {number} midRate
 * @property {number} z
 * @property {number} p
 * @property {number} decodeFailures
 * @property {'PASS'|'FAIL'|'VOID'|'INSUFFICIENT'} verdict
 * @property {string} reason
 */

/**
 * Classify every transition in a captured frame-index stream and decide whether
 * the wrap point behaves differently from the rest of the loop.
 *
 * The comparison — not the absolute anomaly count — is the measurement. USB
 * capture is not frame-locked to the player, so it drops and duplicates frames
 * on its own; those artifacts are uniformly distributed while a real seam is
 * concentrated at the wrap. The mid-loop rate is therefore the noise floor,
 * measured on the same run by the same instrument.
 *
 * @param {(number|null)[]} indices - decoded frame index per captured frame
 * @param {{loopLength?: number, alpha?: number, minWraps?: number}} [opts]
 * @returns {AnalyzeResult}
 */
export function analyze(indices, opts = {}) {
  const alpha = opts.alpha ?? 0.01;
  const minWraps = opts.minWraps ?? 500;

  /** @type {number[]} */
  const good = /** @type {number[]} */ (indices.filter((v) => v !== null));
  if (good.length === 0) throw new Error('no decodable frames');
  const loopLength = opts.loopLength ?? Math.max(...good) + 1;
  const last = loopLength - 1;

  let wrapTransitions = 0, wrapAnomalies = 0;
  let midTransitions = 0, midAnomalies = 0, decodeFailures = 0;
  let lastGood = /** @type {number|null} */ (null);

  for (let i = 0; i + 1 < indices.length; i++) {
    const prev = indices[i];
    const cur = indices[i + 1];
    if (prev !== null) lastGood = prev;

    /** @type {'wrap'|'mid'} */ let bucket;
    let anomalous;

    if (prev === null || cur === null) {
      decodeFailures++;
      bucket = lastGood === last ? 'wrap' : 'mid';
      anomalous = true;
    } else if (cur === prev + 1) {
      bucket = 'mid'; anomalous = false;
    } else if (prev === last && cur === 0) {
      bucket = 'wrap'; anomalous = false;
    } else if (cur < prev || prev === last) {
      bucket = 'wrap'; anomalous = true;
    } else {
      bucket = 'mid'; anomalous = true;
    }

    if (bucket === 'wrap') { wrapTransitions++; if (anomalous) wrapAnomalies++; }
    else { midTransitions++; if (anomalous) midAnomalies++; }
  }

  const wrapRate = wrapTransitions ? wrapAnomalies / wrapTransitions : 0;
  const midRate = midTransitions ? midAnomalies / midTransitions : 0;

  if (wrapTransitions < minWraps) {
    return {
      loopLength, wrapTransitions, wrapAnomalies, midTransitions, midAnomalies,
      wrapRate, midRate, z: 0, p: 1, decodeFailures,
      verdict: 'INSUFFICIENT',
      reason: `only ${wrapTransitions} wrap transitions; need >= ${minWraps}`,
    };
  }

  const { z, p } = twoProportionTest({
    x1: wrapAnomalies, n1: wrapTransitions,
    x2: midAnomalies, n2: midTransitions,
  });

  /** @type {'PASS'|'FAIL'|'VOID'} */
  let verdict;
  let reason;
  if (p >= alpha) {
    verdict = 'PASS';
    reason = `wrap rate ${wrapRate.toFixed(5)} indistinguishable from noise floor ${midRate.toFixed(5)} (p=${p.toFixed(4)})`;
  } else if (wrapRate > midRate) {
    verdict = 'FAIL';
    reason = `wrap rate ${wrapRate.toFixed(5)} exceeds noise floor ${midRate.toFixed(5)} (p=${p.toExponential(2)}) — seam detected`;
  } else {
    verdict = 'VOID';
    reason = `wrap rate ${wrapRate.toFixed(5)} is BELOW noise floor ${midRate.toFixed(5)} (p=${p.toExponential(2)}) — wraps are being misclassified, check loopLength`;
  }

  return {
    loopLength, wrapTransitions, wrapAnomalies, midTransitions, midAnomalies,
    wrapRate, midRate, z, p, decodeFailures, verdict, reason,
  };
}
```

- [ ] **Step 4: Run the tests**

Run: `node --test test/analyze.test.mjs && pnpm typecheck`
Expected: all nine tests PASS, typecheck clean.

- [ ] **Step 5: Write the CLI**

`bin/analyze.mjs`:

```js
#!/usr/bin/env node
// @ts-check
import { readFileSync } from 'node:fs';
import { analyze } from '../lib/analyze.mjs';

const args = process.argv.slice(2);
const get = (/** @type {string} */ flag) => {
  const i = args.indexOf(flag);
  return i === -1 ? undefined : args[i + 1];
};

const file = get('--log');
if (!file) {
  console.error('usage: analyze.mjs --log <indices.txt> [--loop-length N] [--alpha 0.01] [--min-wraps 500] [--json]');
  process.exit(2);
}

const indices = readFileSync(file, 'utf8').split('\n')
  .filter((l) => l.trim() !== '')
  .map((l) => (l.trim() === 'null' ? null : Number(l)));

const result = analyze(indices, {
  loopLength: get('--loop-length') ? Number(get('--loop-length')) : undefined,
  alpha: get('--alpha') ? Number(get('--alpha')) : undefined,
  minWraps: get('--min-wraps') ? Number(get('--min-wraps')) : undefined,
});

if (args.includes('--json')) {
  console.log(JSON.stringify(result, null, 2));
} else {
  console.log(`verdict:      ${result.verdict}`);
  console.log(`reason:       ${result.reason}`);
  console.log(`loop length:  ${result.loopLength} frames`);
  console.log(`wraps:        ${result.wrapAnomalies}/${result.wrapTransitions} anomalous`);
  console.log(`mid-loop:     ${result.midAnomalies}/${result.midTransitions} anomalous (noise floor)`);
  console.log(`decode fails: ${result.decodeFailures}`);
}

// Exit non-zero on anything that is not a clean pass, so shell pipelines and
// the soak runner can branch on it without parsing stdout.
process.exit(result.verdict === 'PASS' ? 0 : 1);
```

- [ ] **Step 6: Commit**

```bash
chmod +x bin/analyze.mjs
git add experiments/2026-08-12-4k-hevc-perfect-loop/lib/analyze.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/bin/analyze.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/analyze.test.mjs
git commit -m "E: 4k-loop-probe — analyzer with noise-floor control and VOID detection"
git push
```

---

### Task 6: Capture — pixels to a frame-index log

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/bin/capture.mjs`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/capture.test.mjs`

**Interfaces:**
- Consumes: `decodeFilter`, `decodeFrame`, `FRAME_BYTES` from `lib/barcode.mjs` (Task 2).
- Produces: `bin/capture.mjs --source <path|avfoundation:N> --height H --out <log> [--frames N] [--fps F]`, writing one decoded index per line (`null` for unreadable frames).

**Why a file source exists alongside the device source:** it makes capture testable with no Cam Link attached, and it gives replay — any recorded clip can be re-run through a revised decoder. That is the same property that motivated splitting capture from analysis in the first place.

- [ ] **Step 1: Write the failing test**

`test/capture.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('capture from a file yields the exact frame-index sequence', () => {
  const dir = mkdtempSync(join(tmpdir(), 'cap-'));
  try {
    const testCard = join(dir, 'm.mkv');
    const burned = join(dir, 'b.mkv');
    const log = join(dir, 'idx.txt');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', '45', '--output', testCard]);
    execFileSync('bash', ['scripts/add-barcode.sh', '--input', testCard, '--output', burned]);

    execFileSync('node', ['bin/capture.mjs', '--source', burned, '--height', '192', '--out', log]);

    const lines = readFileSync(log, 'utf8').trim().split('\n').map(Number);
    assert.equal(lines.length, 45);
    assert.deepEqual(lines, Array.from({ length: 45 }, (_, i) => i));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('--frames stops capture early', () => {
  const dir = mkdtempSync(join(tmpdir(), 'cap-'));
  try {
    const testCard = join(dir, 'm.mkv');
    const burned = join(dir, 'b.mkv');
    const log = join(dir, 'idx.txt');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', '45', '--output', testCard]);
    execFileSync('bash', ['scripts/add-barcode.sh', '--input', testCard, '--output', burned]);
    execFileSync('node', ['bin/capture.mjs', '--source', burned, '--height', '192',
      '--out', log, '--frames', '10']);
    assert.equal(readFileSync(log, 'utf8').trim().split('\n').length, 10);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/capture.test.mjs`
Expected: FAIL — `Cannot find module .../bin/capture.mjs`

- [ ] **Step 3: Write the capture tool**

`bin/capture.mjs`:

```js
#!/usr/bin/env node
// @ts-check
import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { decodeFilter, decodeFrame, FRAME_BYTES } from '../lib/barcode.mjs';

const args = process.argv.slice(2);
const get = (/** @type {string} */ flag) => {
  const i = args.indexOf(flag);
  return i === -1 ? undefined : args[i + 1];
};

const source = get('--source');
const height = Number(get('--height'));
const out = get('--out');
const maxFrames = get('--frames') ? Number(get('--frames')) : Infinity;
const fps = get('--fps');

if (!source || !out || !Number.isFinite(height)) {
  console.error('usage: capture.mjs --source <path|avfoundation:N> --height H --out <log> [--frames N] [--fps F]');
  process.exit(2);
}

/** Build ffmpeg input args for either a file or the Cam Link via avfoundation. */
const inputArgs = source.startsWith('avfoundation:')
  ? ['-f', 'avfoundation', ...(fps ? ['-framerate', fps] : []), '-i', source.slice('avfoundation:'.length)]
  : ['-i', source];

const ff = spawn('ffmpeg', [
  '-hide_banner', '-loglevel', 'error',
  ...inputArgs,
  '-vf', decodeFilter(height),
  '-f', 'rawvideo', '-pix_fmt', 'gray', '-',
], { stdio: ['ignore', 'pipe', 'inherit'] });

const sink = createWriteStream(out);
let pending = Buffer.alloc(0);
let count = 0;

ff.stdout.on('data', (chunk) => {
  pending = pending.length ? Buffer.concat([pending, chunk]) : chunk;
  let offset = 0;
  while (pending.length - offset >= FRAME_BYTES && count < maxFrames) {
    const view = new Uint8Array(pending.buffer, pending.byteOffset + offset, FRAME_BYTES);
    const index = decodeFrame(view);
    sink.write(index === null ? 'null\n' : `${index}\n`);
    offset += FRAME_BYTES;
    count++;
  }
  pending = pending.subarray(offset);
  if (count >= maxFrames) ff.kill('SIGTERM');
});

ff.on('close', () => {
  sink.end(() => {
    process.stderr.write(`captured ${count} frames -> ${out}\n`);
    process.exit(0);
  });
});
```

- [ ] **Step 4: Run the tests**

Run:
```bash
chmod +x bin/capture.mjs
node --test test/capture.test.mjs
pnpm typecheck
```
Expected: both tests PASS, typecheck clean.

- [ ] **Step 5: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/bin/capture.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/capture.test.mjs
git commit -m "E: 4k-loop-probe — capture with file and avfoundation sources"
git push
```

---

### Task 7: The Pi-side probe runner

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/probe.sh`
- Test: `experiments/2026-08-12-4k-hevc-perfect-loop/test/probe.test.mjs`

**Interfaces:**
- Consumes: an encoded asset from Task 4.
- Produces: `scripts/probe.sh --config <loop-file|ab-loop|keep-open> --asset <file> [--vo drm] [--hwdec drm] [--stats <file>] [--dry-run]`. With `--dry-run` it prints the argv, one token per line, and exits 0 without launching mpv.

**Why `--dry-run` is the test surface:** the deliverable of the whole experiment is *an argv*. Testing that each named config emits the intended argv is testing the actual product, and it runs on the Mac with no Pi and no mpv installed.

- [ ] **Step 1: Write the failing test**

`test/probe.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

const argv = (extra) =>
  execFileSync('bash', ['scripts/probe.sh', '--asset', '/tmp/x.mp4', '--dry-run', ...extra])
    .toString().trim().split('\n');

test('loop-file config uses --loop-file=inf and never touches EOF handling flags', () => {
  const a = argv(['--config', 'loop-file']);
  assert.equal(a[0], 'mpv');
  assert.ok(a.includes('--loop-file=inf'));
  assert.ok(a.includes('/tmp/x.mp4'));
  assert.equal(a.some((t) => t.startsWith('--ab-loop')), false);
});

test('ab-loop config seeks before EOF using an explicit b point', () => {
  const a = argv(['--config', 'ab-loop', '--duration', '1.0']);
  assert.ok(a.includes('--ab-loop-a=0'));
  assert.ok(a.some((t) => /^--ab-loop-b=0\.9\d*$/.test(t)), `got ${a.join(' ')}`);
});

test('keep-open config opens an IPC socket for scripted seeking', () => {
  const a = argv(['--config', 'keep-open']);
  assert.ok(a.includes('--keep-open=yes'));
  assert.ok(a.some((t) => t.startsWith('--input-ipc-server=')));
});

test('every config is a kiosk: fullscreen, no OSC, no default bindings', () => {
  for (const cfg of ['loop-file', 'ab-loop', 'keep-open']) {
    const a = argv(['--config', cfg, '--duration', '1.0']);
    assert.ok(a.includes('--fullscreen'), `${cfg} missing --fullscreen`);
    assert.ok(a.includes('--no-osc'), `${cfg} missing --no-osc`);
    assert.ok(a.includes('--no-input-default-bindings'), `${cfg} missing --no-input-default-bindings`);
  }
});

test('vo and hwdec are overridable so the bench can enumerate combinations', () => {
  const a = argv(['--config', 'loop-file', '--vo', 'gpu', '--hwdec', 'v4l2request']);
  assert.ok(a.includes('--vo=gpu'));
  assert.ok(a.includes('--hwdec=v4l2request'));
});

test('an unknown config is rejected rather than silently defaulting', () => {
  assert.throws(() => argv(['--config', 'nonsense']), /unknown config/i);
});

test('ab-loop without --duration is rejected', () => {
  assert.throws(() => argv(['--config', 'ab-loop']), /--duration is required/i);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/probe.test.mjs`
Expected: FAIL — `scripts/probe.sh: No such file or directory`

- [ ] **Step 3: Write the probe runner**

`scripts/probe.sh`:

```bash
#!/usr/bin/env bash
# Launch one named mpv loop configuration on the Pi.
#
# The three configurations are the three mechanisms from DOSSIER-2 §5:
#   loop-file  - mpv's own looping; goes through EOF handling
#   ab-loop    - seeks before EOF is ever reached, so EOF handling is bypassed
#   keep-open  - holds the file open and seeks under external script control
#
# --dry-run prints the argv instead of running it. That argv IS the deliverable
# of this experiment: milestone 2's mpv.py play() shells out to exactly this.
set -euo pipefail

usage() {
  echo "usage: $0 --config <loop-file|ab-loop|keep-open> --asset <file> [--duration SEC] [--vo VO] [--hwdec HW] [--stats FILE] [--dry-run]" >&2
  exit 2
}

CONFIG=""; ASSET=""; DURATION=""; VO="drm"; HWDEC="drm"; STATS=""; DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --config)   CONFIG="$2";   shift 2 ;;
    --asset)    ASSET="$2";    shift 2 ;;
    --duration) DURATION="$2"; shift 2 ;;
    --vo)       VO="$2";       shift 2 ;;
    --hwdec)    HWDEC="$2";    shift 2 ;;
    --stats)    STATS="$2";    shift 2 ;;
    --dry-run)  DRY=1;         shift ;;
    *) usage ;;
  esac
done
[ -n "$CONFIG" ] && [ -n "$ASSET" ] || usage

# Kiosk flags shared by every configuration. A visible OSC or a stray keybinding
# would be indistinguishable from a player artifact in the capture.
ARGS=(mpv
  "--vo=${VO}"
  "--hwdec=${HWDEC}"
  --fullscreen
  --no-osc
  --no-input-default-bindings
  --no-terminal
  --msg-level=all=warn
)

[ -n "$STATS" ] && ARGS+=("--dump-stats=${STATS}")

case "$CONFIG" in
  loop-file)
    ARGS+=(--loop-file=inf)
    ;;
  ab-loop)
    [ -n "$DURATION" ] || { echo "--duration is required for the ab-loop config" >&2; exit 2; }
    # Stop one hundredth of a second short of the end so the seek happens before
    # EOF is ever reached — the point of this configuration.
    B="$(awk -v d="$DURATION" 'BEGIN { printf "%.3f", d - 0.01 }')"
    ARGS+=(--loop-file=inf "--ab-loop-a=0" "--ab-loop-b=${B}")
    ;;
  keep-open)
    ARGS+=(--keep-open=yes "--input-ipc-server=/tmp/mpv-probe.sock")
    ;;
  *)
    echo "unknown config: $CONFIG" >&2
    exit 2
    ;;
esac

ARGS+=("$ASSET")

if [ "$DRY" -eq 1 ]; then
  printf '%s\n' "${ARGS[@]}"
  exit 0
fi

exec "${ARGS[@]}"
```

- [ ] **Step 4: Run shellcheck and the tests**

Run:
```bash
chmod +x scripts/probe.sh
shellcheck scripts/probe.sh
node --test test/probe.test.mjs
```
Expected: shellcheck silent, all seven tests PASS.

- [ ] **Step 5: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/scripts/probe.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/probe.test.mjs
git commit -m "E: 4k-loop-probe — mpv config matrix runner with dry-run argv output"
git push
```

---

### Task 8: End-to-end dress rehearsal and the soak runner

The final task proves the *instrument* before any Pi is flashed: generate, burn, encode, "capture" from the file, analyze — and confirm the pipeline reports PASS on a known-good file and FAIL on a file with a planted seam. This is SPEC.md Control 2 executed against the real pipeline rather than against synthetic logs.

**Files:**
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/soak.sh`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/test/e2e.test.mjs`
- Create: `experiments/2026-08-12-4k-hevc-perfect-loop/README.md`

**Interfaces:**
- Consumes: every unit from Tasks 1-7.
- Produces: `scripts/soak.sh --source <path|avfoundation:N> --height H --loop-length N --outdir <dir> [--chunk-frames 18000]`, which captures in chunks, analyzes each, appends a one-line summary per chunk, and exits non-zero if any chunk is not PASS.

- [ ] **Step 1: Write the failing end-to-end test**

`test/e2e.test.mjs`:

```js
// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const LOOP = 30;
const WRAPS = 600;

function buildLoopedCapture(dir) {
  const testCard = join(dir, 'm.mkv');
  const burned = join(dir, 'b.mkv');
  const log = join(dir, 'idx.txt');
  execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
    '--fps', '30', '--frames', String(LOOP), '--output', testCard]);
  execFileSync('bash', ['scripts/add-barcode.sh', '--input', testCard, '--output', burned]);
  execFileSync('node', ['bin/capture.mjs', '--source', burned, '--height', '192', '--out', log]);
  return readFileSync(log, 'utf8').trim().split('\n');
}

test('E2E: a genuinely seamless source is reported PASS by the real pipeline', () => {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-'));
  try {
    const one = buildLoopedCapture(dir);
    assert.deepEqual(one.map(Number), Array.from({ length: LOOP }, (_, i) => i));

    // Concatenate the single-loop capture WRAPS times: exactly what a perfect
    // player would put on the wire.
    const full = join(dir, 'full.txt');
    writeFileSync(full, Array.from({ length: WRAPS }, () => one.join('\n')).join('\n') + '\n');

    const out = execFileSync('node', ['bin/analyze.mjs', '--log', full,
      '--loop-length', String(LOOP)]).toString();
    assert.match(out, /verdict:\s+PASS/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('E2E: a planted seam is caught, and the exit code is non-zero', () => {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-'));
  try {
    const one = buildLoopedCapture(dir);
    // Plant a held final frame at every wrap — the classic seam signature.
    const seamed = Array.from({ length: WRAPS }, () => [...one, one[one.length - 1]].join('\n')).join('\n');
    const full = join(dir, 'seam.txt');
    writeFileSync(full, seamed + '\n');

    let code = 0;
    let out = '';
    try {
      out = execFileSync('node', ['bin/analyze.mjs', '--log', full,
        '--loop-length', String(LOOP)]).toString();
    } catch (e) {
      code = e.status;
      out = e.stdout.toString();
    }
    assert.equal(code, 1, 'analyze must exit non-zero on a seam');
    assert.match(out, /verdict:\s+FAIL/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test test/e2e.test.mjs`
Expected: FAIL — `scripts/soak.sh` is not yet needed by these two, so if they already pass, the units from Tasks 1-7 are working; proceed to Step 3. If they fail, fix the reported unit before continuing.

- [ ] **Step 3: Write the soak runner**

`scripts/soak.sh`:

```bash
#!/usr/bin/env bash
# Long-run capture: chunk, analyze each chunk, keep only the summary.
#
# Storing the frame-index log rather than video is what makes a 24h soak
# continuously measured instead of sampled: 24h at 30fps is ~2.6M integers,
# which is nothing, while the equivalent video is unmanageable.
set -euo pipefail

usage() {
  echo "usage: $0 --source <path|avfoundation:N> --height H --loop-length N --outdir <dir> [--chunk-frames 18000] [--chunks 0]" >&2
  exit 2
}

SOURCE=""; HEIGHT=""; LOOP_LENGTH=""; OUTDIR=""; CHUNK_FRAMES=18000; CHUNKS=0
while [ $# -gt 0 ]; do
  case "$1" in
    --source)       SOURCE="$2";       shift 2 ;;
    --height)       HEIGHT="$2";       shift 2 ;;
    --loop-length)  LOOP_LENGTH="$2";  shift 2 ;;
    --outdir)       OUTDIR="$2";       shift 2 ;;
    --chunk-frames) CHUNK_FRAMES="$2"; shift 2 ;;
    --chunks)       CHUNKS="$2";       shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$SOURCE" ] && [ -n "$HEIGHT" ] && [ -n "$LOOP_LENGTH" ] && [ -n "$OUTDIR" ] || usage

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$OUTDIR"
SUMMARY="$OUTDIR/summary.tsv"
[ -f "$SUMMARY" ] || printf 'chunk\ttimestamp\tverdict\twrap_anom\twrap_n\tmid_anom\tmid_n\tp\n' > "$SUMMARY"

FAILED=0
i=0
while [ "$CHUNKS" -eq 0 ] || [ "$i" -lt "$CHUNKS" ]; do
  LOG="$OUTDIR/chunk-$(printf '%05d' "$i").txt"
  node "$HERE/../bin/capture.mjs" --source "$SOURCE" --height "$HEIGHT" \
    --out "$LOG" --frames "$CHUNK_FRAMES"

  if RESULT="$(node "$HERE/../bin/analyze.mjs" --log "$LOG" \
      --loop-length "$LOOP_LENGTH" --json)"; then
    VERDICT=PASS
  else
    VERDICT="$(printf '%s' "$RESULT" | node -e \
      'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).verdict))')"
    FAILED=1
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$i" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$VERDICT" \
    "$(printf '%s' "$RESULT" | node -pe 'JSON.parse(require("fs").readFileSync(0,"utf8")).wrapAnomalies')" \
    "$(printf '%s' "$RESULT" | node -pe 'JSON.parse(require("fs").readFileSync(0,"utf8")).wrapTransitions')" \
    "$(printf '%s' "$RESULT" | node -pe 'JSON.parse(require("fs").readFileSync(0,"utf8")).midAnomalies')" \
    "$(printf '%s' "$RESULT" | node -pe 'JSON.parse(require("fs").readFileSync(0,"utf8")).midTransitions')" \
    "$(printf '%s' "$RESULT" | node -pe 'JSON.parse(require("fs").readFileSync(0,"utf8")).p')" \
    >> "$SUMMARY"

  # Keep the raw log only when something went wrong; a clean 24h run would
  # otherwise leave thousands of uninteresting files behind.
  [ "$VERDICT" = "PASS" ] && rm -f "$LOG"

  i=$((i + 1))
done

exit "$FAILED"
```

- [ ] **Step 4: Write the README**

`README.md`:

````markdown
# 4K HEVC Perfect Loop — probe harness

Measurement harness for the dex seamless-loop experiment.
Design: [SPEC.md](SPEC.md) · Running record: [LOG-4k-hevc-perfect-loop.md](LOG-4k-hevc-perfect-loop.md)

## Install

```bash
pnpm install          # devDependencies only; the harness itself has zero runtime deps
pnpm test             # unit + end-to-end tests, no hardware required
pnpm typecheck
```

Requires `ffmpeg` (with `libx265`), `node` >= 22, and `shellcheck` for development.

## Generate assets

```bash
# 1s 4K30 test card, one exact rotation over the loop
mkdir -p out/lossless
scripts/make-test-card.sh --width 3840 --height 2160 --fps 30 --frames 30 \
  --output out/lossless/test-card-1s-2160p30.mkv
scripts/add-barcode.sh --input out/lossless/test-card-1s-2160p30.mkv \
  --output out/lossless/test-card-1s-2160p30-barcoded.mkv
scripts/encode-variants.sh --input out/lossless/test-card-1s-2160p30-barcoded.mkv \
  --outdir out --name dex-test-card-1s-2160p30 --fps 30 --bitrate 40M
```

For the `hello_video` positive control, encode the 1080p variant with `--h264`.

## Run a probe (on the Pi)

```bash
scripts/probe.sh --config loop-file --asset dex-test-card-1s-2160p30.mp4 --stats /tmp/mpv-stats.txt
scripts/probe.sh --config loop-file --asset dex-test-card-1s-2160p30.mp4 --dry-run   # print the argv
```

## Measure (on the Mac, Cam Link attached)

```bash
bin/capture.mjs --source avfoundation:0 --height 2160 --out out/idx.txt --frames 18000
bin/analyze.mjs --log out/idx.txt --loop-length 30
```

## Soak

```bash
scripts/soak.sh --source avfoundation:0 --height 2160 --loop-length 30 --outdir out/soak
```

Writes one summary row per chunk to `out/soak/summary.tsv` and keeps raw logs only for
non-passing chunks.

## Reading a verdict

| Verdict | Meaning |
|---|---|
| `PASS` | Wrap anomaly rate indistinguishable from the mid-loop noise floor |
| `FAIL` | Wrap rate significantly exceeds the noise floor — a seam |
| `VOID` | Wrap rate is significantly *below* the floor — wraps are misclassified, check `--loop-length` |
| `INSUFFICIENT` | Fewer than 500 wrap transitions; capture longer |
````

- [ ] **Step 5: Run everything**

Run:
```bash
shellcheck scripts/*.sh
pnpm test
pnpm typecheck
```
Expected: shellcheck silent on all five scripts, the full suite PASSes, typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/scripts/soak.sh \
        experiments/2026-08-12-4k-hevc-perfect-loop/test/e2e.test.mjs \
        experiments/2026-08-12-4k-hevc-perfect-loop/README.md
git commit -m "E: 4k-loop-probe — soak runner, end-to-end dress rehearsal, README"
git push
```

- [ ] **Step 7: Advance the experiment to SETUP**

The harness is now validated without hardware. Update `LOG-4k-hevc-perfect-loop.md`:
set `phase: SETUP` in the frontmatter, fill the **Environment** section (Pi OS image and version,
`mpv --version`, `ffmpeg -version` and whether it is the `+rpt2` build, `uname -a`, board
revisions, display mode, Cam Link firmware, HDMI cable), and append a dated Running Log entry
recording that the instrument passed its own controls before touching hardware.

```bash
git add experiments/2026-08-12-4k-hevc-perfect-loop/LOG-4k-hevc-perfect-loop.md
git commit -m "D: 4k-loop-probe — advance to SETUP, harness validated offline"
git push
```

---

## Bench procedure (after the harness is built)

Not code, but the order matters and is pre-committed so it is not improvised at 23:00.

1. **Flash** Pi OS Trixie Lite (64-bit) via Raspberry Pi Imager. `apt install mpv`. Record versions in the LOG Environment section.
2. **Control 3 first.** Run the `hello_video` positive control on the dexOS card with the 1080p H.264 barcoded asset. The analyzer must report `PASS`. **If it does not, stop** — the rig is wrong and no other number from it means anything.
3. **Pi 5 + `test-card-1s-2160p30` + `--config loop-file`.** Capture 600+ wraps (~10 min). Analyze.
4. On failure, walk the matrix: `ab-loop`, then `keep-open`, then the `--vo` / `--hwdec` combinations that Trixie's mpv actually offers.
5. Once a config passes: repeat on **Pi 4**, then `test-card-10s-2160p30`, then the 4K60 stretch check.
6. **24h soak** on the winner via `soak.sh`, plus mpv `--dump-stats` for the in-band signal.
7. **Eyeball A/B** against the dexOS card on the projector (sequential with capture — the Cam Link has no passthrough).
8. Record the winning argv verbatim in `SPEC.md` findings and the LOG.

**Stop condition:** 2 bench days with no config reaching baseline on either board → declare REFUTED, escalate per SPEC.md §6. Do not try more mpv configurations.

## Self-Review

**Spec coverage.** SPEC.md §2's seven units all have tasks: `make-test-card.sh` (3), `add-barcode.sh` (2), `encode-variants.sh` (4), `probe.sh` (7), `capture.mjs` (6), `analyze.mjs` (5), `soak.sh` (8). §3 assets: generator (3), barcode (2), variants and sidecars (4). §4 measurement: in-band via `--stats` (7), out-of-band (6), Control 1 noise floor (5), Control 2 injection (5 and 8), Control 3 `hello_video` (4 emits the asset, bench procedure step 2 runs it). §5 matrix and order: task 7 plus the bench procedure. §6 escalation and §7 roadmap are decisions, not code — they live in SPEC.md and need no task.

**Gap found and closed:** the 4K60 stretch check needs 1080p60 capture while the source is 4K60. `capture.mjs` takes `--height` explicitly rather than probing the source, so the operator supplies the *capture* height, not the source height — no code change needed, but it is a real trap, so it is called out here and in the bench procedure.

**Type consistency.** `decodeFrame` returns `number | null` everywhere; `analyze` accepts `(number|null)[]` and `capture.mjs` writes the literal `null` that `bin/analyze.mjs` parses back to `null`. `barHeight`/`burnFilter`/`decodeFilter` share one definition in `lib/barcode.mjs`, consumed by shell through `bin/barcode-filter.mjs`. `AnalyzeResult` field names are identical in the typedef, the CLI, and `soak.sh`'s JSON extraction.

**Placeholder scan.** No TBDs. The only deferred item is the exact `--vo`/`--hwdec` enumeration, which is deferred *by design* — it depends on what Trixie's mpv reports at the bench — and `probe.sh` takes both as parameters so no code changes when the answer is known.
