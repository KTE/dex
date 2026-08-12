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
  assert.equal(CELLS, 18);
  assert.equal(CELLS, 2 + DATA_BITS);
  assert.equal(DECODE_W, CELLS * 4);
  assert.equal(FRAME_BYTES, DECODE_W * DECODE_H);
});

test('the barcode lands centred and exactly on the test card grid', () => {
  // The card's grid was measured from a real frame: 50px cells at 1080p,
  // lines at x = 10 + 50k and y = 40 + 50k.
  const hd = barcodeRect(1920, 1080);
  assert.deepEqual(hd, { x: 510, y: 940, w: 900, h: 50, cellW: 50 });
  assert.equal(hd.x + hd.w / 2, 960, 'must be horizontally centred');
  assert.equal((hd.x - 10) % 50, 0, 'left edge must sit on a grid line');
  assert.equal((hd.y - 40) % 50, 0, 'top edge must sit on a grid line');
  assert.equal(hd.w / hd.cellW, CELLS, 'must be a whole number of grid cells wide');

  // At 4K the card is scaled 2x, so the grid is 100px and the same holds.
  const uhd = barcodeRect(3840, 2160);
  assert.deepEqual(uhd, { x: 1020, y: 1880, w: 1800, h: 100, cellW: 100 });
  assert.equal(uhd.x + uhd.w / 2, 1920, 'must be horizontally centred');
  assert.equal((uhd.x - 20) % 100, 0, 'left edge must sit on the 4K grid');
  assert.equal((uhd.y - 80) % 100, 0, 'top edge must sit on the 4K grid');
});

test('the barcode clears the frame edges — no longer covering the checkerboard border', () => {
  for (const [w, h] of [[1920, 1080], [3840, 2160]]) {
    const r = barcodeRect(w, h);
    assert.ok(r.x > 0 && r.x + r.w < w, 'must not touch the left or right edge');
    assert.ok(r.y > h * 0.75, 'must sit in the lower quarter, where it was marked');
    assert.ok(r.y + r.h < h, 'must not touch the bottom edge');
  }
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
