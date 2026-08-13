#!/usr/bin/env node
// @ts-check
// Measure DISPLAY timing from a recorded capture. Touches no hardware.
//
// This is the gold-standard instrument: it measures what actually reached the
// screen, using the recording's own timestamps, rather than asking the player
// how it thinks it is doing. That distinction is not academic -- on 2026-08-13
// mpv reported frame-drop-count=0 while presenting ~28.3 unique frames per
// second into a 30 Hz mode. It dropped nothing; it HELD frames. A player's drop
// counters cannot see that, and neither can a live index stream with no clock.
//
// The primary measurement is the LOOP PERIOD: wall-clock time between successive
// wraps of the barcode. For a 90-frame loop at 30 fps every period must be
// exactly 3.000 s. Periods are independent of the capture device's own frame
// rate, which is why this survives the Cam Link's ~27 fps sampling of a 30 fps
// display -- a sampling deficit shifts WHICH frames are seen, not WHEN the wrap
// happens.
//
// Geometry is passed in rather than imported from lib/barcode.mjs on purpose:
// a recording outlives the geometry it was made under, and on 2026-08-13 a
// concurrent session moved the barcode mid-experiment. Being able to re-decode
// old evidence with old geometry is the whole point of recording.
import { spawn } from 'node:child_process';

const args = process.argv.slice(2);
/** @type {(flag: string) => string | undefined} */
const get = (flag) => {
  const i = args.indexOf(flag);
  return i === -1 ? undefined : args[i + 1];
};

const clip = get('--clip');
const loopLength = Number(get('--loop-length') ?? 90);
const srcFps = Number(get('--fps') ?? 30);
// Barcode rectangle WITHIN THE RECORDED BAND, in pixels.
const barX = Number(get('--bar-x') ?? 1020);
const barY = Number(get('--bar-y') ?? 152);
const barW = Number(get('--bar-w') ?? 1800);
const barH = Number(get('--bar-h') ?? 100);

if (!clip) {
  console.error('usage: analyze-timing.mjs --clip <file> [--loop-length N] [--fps F]');
  console.error('       [--bar-x X --bar-y Y --bar-w W --bar-h H]  (pixels within the recorded band)');
  process.exit(2);
}

const CELLS = 18;
const DATA_BITS = 16;
const CELL_PX = 4;
const DECODE_W = CELLS * CELL_PX;
const DECODE_H = 8;
const FRAME_BYTES = DECODE_W * DECODE_H;
const SYNC_MIN_DELTA = 40;

/** @param {Uint8Array} gray @returns {number | null} */
function decodeFrame(gray) {
  const row = Math.floor(DECODE_H / 2);
  /** @type {(cell: number) => number} */
  const sample = (cell) => gray[row * DECODE_W + cell * CELL_PX + Math.floor(CELL_PX / 2)];
  const white = sample(0);
  const black = sample(1);
  if (white - black < SYNC_MIN_DELTA) return null;
  const threshold = (white + black) / 2;
  let value = 0;
  for (let k = 0; k < DATA_BITS; k++) if (sample(2 + k) > threshold) value |= 1 << k;
  return value;
}

/**
 * Collect stdout of a command as a string.
 * @param {string} cmd
 * @param {string[]} argv
 * @returns {Promise<string>}
 */
const run = (cmd, argv) =>
  new Promise((resolve, reject) => {
    const p = spawn(cmd, argv, { stdio: ['ignore', 'pipe', 'ignore'] });
    let out = '';
    p.stdout.on('data', (c) => (out += c));
    p.on('close', (code) => (code === 0 ? resolve(out) : reject(new Error(`${cmd} exited ${code}`))));
  });

// Presentation timestamps, one per frame, in capture order.
const ptsText = await run('ffprobe', [
  '-v', 'error', '-select_streams', 'v:0',
  '-show_entries', 'frame=pts_time', '-of', 'csv=p=0', clip,
]);
// Trim BEFORE converting, and drop empty lines explicitly: Number('') is 0, and
// Number.isFinite(0) is true, so a trailing blank line would silently become a
// timestamp of zero at the end of the array and collapse the measured span.
const pts = ptsText
  .split('\n')
  .map((s) => s.trim())
  .filter((s) => s.length > 0)
  .map(Number)
  .filter((v) => Number.isFinite(v));

// Barcode indices, one per frame, same order.
const ff = spawn('ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-i', clip,
  '-vf', `crop=${barW}:${barH}:${barX}:${barY},scale=${DECODE_W}:${DECODE_H}:flags=area,format=gray`,
  '-f', 'rawvideo', '-pix_fmt', 'gray', '-',
], { stdio: ['ignore', 'pipe', 'inherit'] });

/** @type {(number | null)[]} */
const indices = [];
/** @type {Uint8Array} */
let pending = Buffer.alloc(0);
for await (const chunk of ff.stdout) {
  pending = pending.length ? Buffer.concat([pending, chunk]) : chunk;
  let off = 0;
  while (pending.length - off >= FRAME_BYTES) {
    indices.push(decodeFrame(pending.subarray(off, off + FRAME_BYTES)));
    off += FRAME_BYTES;
  }
  pending = pending.subarray(off);
}

const n = Math.min(pts.length, indices.length);
const decoded = indices.slice(0, n);
const ok = decoded.filter((v) => v !== null).length;

// Wrap times: interpolate nothing, just take the timestamp of the first frame
// of each new cycle. A wrap is any backwards step in the index.
/** @type {number[]} */
const wrapTimes = [];
for (let i = 1; i < n; i++) {
  const a = decoded[i - 1];
  const b = decoded[i];
  if (a === null || b === null) continue;
  if (b < a) wrapTimes.push(pts[i]);
}

/** @type {number[]} */
const periods = [];
for (let i = 1; i < wrapTimes.length; i++) periods.push(wrapTimes[i] - wrapTimes[i - 1]);

const expected = loopLength / srcFps;
const mean = periods.length ? periods.reduce((s, x) => s + x, 0) / periods.length : NaN;
const sorted = [...periods].sort((a, b) => a - b);
const median = sorted.length ? sorted[Math.floor(sorted.length / 2)] : NaN;

/** @type {Record<string, number>} */
const steps = {};
for (let i = 1; i < n; i++) {
  const a = decoded[i - 1];
  const b = decoded[i];
  if (a === null || b === null) continue;
  const s = ((b - a) % loopLength + loopLength) % loopLength;
  steps[String(s)] = (steps[String(s)] ?? 0) + 1;
}

const span = n > 1 ? pts[n - 1] - pts[0] : NaN;
console.log(`clip              : ${clip}`);
console.log(`captured frames   : ${n}   decoded ${ok}   undecodable ${n - ok}`);
console.log(`capture span      : ${span.toFixed(3)} s  (capture rate ${(n / span).toFixed(2)} fps)`);
console.log(`wraps observed    : ${wrapTimes.length}`);
console.log('');
console.log(`expected period   : ${expected.toFixed(4)} s  (${loopLength} frames @ ${srcFps} fps)`);
console.log(`measured mean     : ${mean.toFixed(4)} s   -> ${(expected / mean).toFixed(4)}x realtime`);
console.log(`measured median   : ${median.toFixed(4)} s   -> ${(expected / median).toFixed(4)}x realtime`);
if (periods.length) {
  console.log(`period min / max  : ${Math.min(...periods).toFixed(4)} / ${Math.max(...periods).toFixed(4)} s`);
  console.log(`implied display   : ${(loopLength / mean).toFixed(2)} fps`);
}
console.log('');
console.log('index step histogram (capture-rate artefacts included):');
for (const [s, c] of Object.entries(steps).sort((a, b) => b[1] - a[1]).slice(0, 6)) {
  console.log(`  +${s.padStart(2)} : ${c}`);
}

// Exit non-zero when the loop period is off by more than 1%, so this can gate.
process.exit(Number.isFinite(mean) && Math.abs(mean - expected) / expected <= 0.01 ? 0 : 1);
