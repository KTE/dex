#!/usr/bin/env node
// @ts-check
// Build the background matte from the ANIMATED card, not the static artwork.
//
// WHY THIS REPLACES make-matte.sh
// The static PNG's circle outline is r636-642, but the animated ring drawn over
// it in After Effects spans r628-652 — wider on both sides. A matte keyed from
// the PNG therefore calls r642-652 "background", and the compositor paints
// clouds and grain over the outer edge of the animated ring at every angle.
// Measured 2026-08-13: ~13 differing pixels per angle, all 360 degrees.
//
// A static matte cannot describe an animated card. So this walks every frame and
// keeps a pixel only if it is background in ALL of them — the intersection, not
// a sample. Anything that moves through a pixel at any point in the loop
// (the sweep ring, the rotating hands, the timecode digits) excludes it forever.
//
// That conservative choice is the right one here: texturing a measurement
// element corrupts what it measures, while failing to texture a few background
// pixels costs nothing.
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const args = process.argv.slice(2);
/** @type {(f: string) => string | undefined} */
const get = (f) => { const i = args.indexOf(f); return i === -1 ? undefined : args[i + 1]; };

const input = get('--input');
const out = get('--output');
const width = Number(get('--width') ?? 3840);
const height = Number(get('--height') ?? 2160);
/** Background field value; the dex test card's grid is exactly RGB(211,211,211). */
const BG = Number(get('--bg') ?? 211);

if (!input || !out) {
  console.error('usage: make-matte-from-video.mjs --input <video> --output <matte.pgm> [--width W] [--height H] [--bg 211]');
  process.exit(2);
}

const N = width * height;
/** 1 = still background in every frame seen so far. */
const mask = new Uint8Array(N).fill(1);

const ff = spawn('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', input,
  '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'], { stdio: ['ignore', 'pipe', 'inherit'] });

const FRAME = N * 3;
/** @type {Uint8Array} */
let pending = Buffer.alloc(0);
let frames = 0;

ff.stdout.on('data', (/** @type {Buffer} */ chunk) => {
  pending = pending.length ? Buffer.concat([pending, chunk]) : chunk;
  while (pending.length >= FRAME) {
    const f = pending.subarray(0, FRAME);
    for (let p = 0, i = 0; p < N; p++, i += 3) {
      if (mask[p] && !(f[i] === BG && f[i + 1] === BG && f[i + 2] === BG)) mask[p] = 0;
    }
    frames++;
    pending = pending.subarray(FRAME);
  }
});

ff.on('close', () => {
  // Binary PGM (P5): trivially readable by ffmpeg, no encoder dependency.
  const header = Buffer.from(`P5\n${width} ${height}\n255\n`, 'ascii');
  const body = Buffer.alloc(N);
  let kept = 0;
  for (let p = 0; p < N; p++) { if (mask[p]) { body[p] = 255; kept++; } }
  writeFileSync(out, Buffer.concat([header, body]));
  process.stderr.write(
    `matte from ${frames} frames: ${(100 * kept / N).toFixed(1)}% background -> ${out}\n`);
});
