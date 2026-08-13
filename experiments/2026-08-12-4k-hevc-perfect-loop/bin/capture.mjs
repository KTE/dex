#!/usr/bin/env node
// @ts-check
// Convert pixels into a stream of frame indices. Decides nothing.
//
// The file source is not just for testing: it gives replay, so any recorded
// clip can be re-run through a revised decoder without re-capturing. That is
// the same property that motivated splitting capture from analysis.
import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { decodeFilter, decodeFrame, FRAME_BYTES } from '../lib/barcode.mjs';

const args = process.argv.slice(2);
/** @type {(flag: string) => string | undefined} */
const get = (flag) => {
  const i = args.indexOf(flag);
  return i === -1 ? undefined : args[i + 1];
};

const source = get('--source');
const out = get('--out');
const framesArg = get('--frames');
const maxFrames = framesArg ? Number(framesArg) : Infinity;
const fps = get('--fps');

if (!source || !out) {
  console.error('usage: capture.mjs --source <path|avfoundation:N> --out <log> [--frames N] [--fps F]');
  console.error('  No resolution is needed: the barcode rectangle is expressed as fractions');
  console.error('  of the frame, so the decode filter works at any capture resolution.');
  process.exit(2);
}

/** ffmpeg input args for either a file or a capture device via avfoundation. */
const inputArgs = source.startsWith('avfoundation:')
  ? ['-f', 'avfoundation', ...(fps ? ['-framerate', fps] : []), '-i', source.slice('avfoundation:'.length)]
  : ['-i', source];

const ff = spawn('ffmpeg', [
  '-hide_banner', '-loglevel', 'error',
  ...inputArgs,
  // Without this, ffmpeg produces a CONSTANT-frame-rate stream and duplicates
  // frames to fill it. Against a live capture device, with nothing pacing the
  // pipeline, it duplicates as fast as the consumer can read. Measured
  // 2026-08-13: 300 "frames" arrived in 0.887 s (~340 fps from a 27 fps device),
  // every one of them the same frame, decoding to a single constant index.
  // That is a silently frozen capture -- it looks like a successful run, and
  // every measurement taken from it is worthless. passthrough emits exactly the
  // frames the device delivers, at the device's own pace.
  '-fps_mode', 'passthrough',
  '-vf', decodeFilter(),
  '-f', 'rawvideo', '-pix_fmt', 'gray', '-',
], { stdio: ['ignore', 'pipe', 'inherit'] });

const sink = createWriteStream(out);
// Uint8Array, not Buffer: this code needs nothing Buffer-specific, and typing it
// as Buffer<ArrayBuffer> makes subarray() reassignment a type error.
/** @type {Uint8Array} */
let pending = Buffer.alloc(0);
let count = 0;

ff.stdout.on('data', (/** @type {Buffer} */ chunk) => {
  pending = pending.length ? Buffer.concat([pending, chunk]) : chunk;
  let offset = 0;
  while (pending.length - offset >= FRAME_BYTES && count < maxFrames) {
    // subarray, not new Uint8Array(buffer, ...): Buffer.buffer is ArrayBufferLike
    // and may be a SharedArrayBuffer, which the Uint8Array overload rejects.
    // Buffer already is a Uint8Array, so this is both simpler and type-correct.
    const index = decodeFrame(pending.subarray(offset, offset + FRAME_BYTES));
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
