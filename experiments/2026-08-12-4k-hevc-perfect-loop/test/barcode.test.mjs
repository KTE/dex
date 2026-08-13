// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  CELLS, DATA_BITS, DECODE_W, DECODE_H, FRAME_BYTES,
  barcodeRect, burnFilter, decodeFilter, decodeFrame,
} from '../lib/barcode.mjs';

/**
 * Build a synthetic decode-sized gray frame encoding `value`.
 * @param {number} value
 * @param {{white?: number, black?: number}} [levels]
 * @returns {Uint8Array}
 */
function synthFrame(value, { white = 235, black = 16 } = {}) {
  const f = new Uint8Array(FRAME_BYTES).fill(black);
  /** @type {(i: number, level: number) => void} */
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
  assert.equal(CELLS, 14);
  assert.equal(CELLS, 2 + DATA_BITS);
  assert.equal(DECODE_W, CELLS * 4);
  assert.equal(FRAME_BYTES, DECODE_W * DECODE_H);
});

test('the barcode fits inside the card label bar, left of the label text', () => {
  // Measured from the rendered 4K card, 2026-08-13:
  //   black bar   x 921..2918   y 1880..1980
  //   label text  x 2075..2758
  const BAR = { x0: 921, x1: 2918, y0: 1880, y1: 1980 };
  const TEXT_X0 = 2075;

  const uhd = barcodeRect(3840, 2160);
  assert.deepEqual(uhd, { x: 941, y: 1884, w: 1092, h: 92, cellW: 78 });
  assert.ok(uhd.x > BAR.x0, 'left edge inside the bar');
  assert.ok(uhd.x + uhd.w < TEXT_X0, 'must end before the label text starts');
  assert.ok(uhd.y > BAR.y0 && uhd.y + uhd.h < BAR.y1, 'vertically inset in the bar');
  assert.equal(uhd.cellW % 1, 0, 'cells must be whole pixels at 4K');

  // 1080p is the same card at half scale, so the fractions carry over.
  const hd = barcodeRect(1920, 1080);
  assert.deepEqual(hd, { x: 471, y: 942, w: 546, h: 46, cellW: 39 });
  assert.equal(hd.cellW % 1, 0, 'cells must be whole pixels at 1080p too');
  assert.ok(hd.x > BAR.x0 / 2, 'left edge inside the half-scale bar');
  assert.ok(hd.x + hd.w < TEXT_X0 / 2, 'must end before the half-scale label text');
});

test('the barcode clears the frame edges', () => {
  for (const [w, h] of [[1920, 1080], [3840, 2160]]) {
    const r = barcodeRect(w, h);
    assert.ok(r.x > 0 && r.x + r.w < w, 'must not touch the left or right edge');
    assert.ok(r.y > h * 0.75, 'must sit in the lower quarter, in the label bar');
    assert.ok(r.y + r.h < h, 'must not touch the bottom edge');
  }
});

test('decodeFrame round-trips synthetic values', () => {
  // 12 data bits: 0..4095. 4095 is the maximum representable index.
  for (const v of [0, 1, 2, 255, 256, 2048, 4095]) {
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

test('the filters are resolution-independent expressions', () => {
  // No dimensions are passed, and none appear in the output — this is what lets
  // capture.mjs and add-barcode.sh work without a --height flag.
  const burn = burnFilter();
  const dec = decodeFilter();
  for (const f of [burn, dec]) {
    assert.ok(f.includes('iw'), 'must be expressed in frame-relative units');
    assert.ok(f.includes('ih'), 'must be expressed in frame-relative units');
    assert.equal(/\b1920\b|\b1080\b|\b3840\b|\b2160\b/.test(f), false,
      'must not hardcode any resolution');
  }
});

test('burn then decode round-trips through real ffmpeg at 1080p', () => {
  const dir = mkdtempSync(join(tmpdir(), 'barcode-'));
  try {
    const src = join(dir, 'src.mkv');
    const burned = join(dir, 'burned.mkv');
    const N = 40;
    // Real 16:9 dimensions, so the grid alignment under test is the real one.
    execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi',
      '-i', `color=c=gray:s=1920x1080:r=30:d=${N / 30}`, '-c:v', 'ffv1', '-y', src]);
    execFileSync('bash', ['scripts/add-barcode.sh', '--input', src, '--output', burned]);

    const raw = execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', burned,
      '-vf', decodeFilter(), '-f', 'rawvideo', '-pix_fmt', 'gray', '-'],
      { maxBuffer: 64 * 1024 * 1024 });

    const frames = Math.floor(raw.length / FRAME_BYTES);
    assert.ok(frames >= N, `expected >= ${N} frames, got ${frames}`);
    const decoded = [];
    for (let i = 0; i < N; i++) {
      decoded.push(decodeFrame(raw.subarray(i * FRAME_BYTES, (i + 1) * FRAME_BYTES)));
    }
    assert.deepEqual(decoded, Array.from({ length: N }, (_, i) => i));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
