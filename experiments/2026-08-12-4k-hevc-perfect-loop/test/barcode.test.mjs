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
