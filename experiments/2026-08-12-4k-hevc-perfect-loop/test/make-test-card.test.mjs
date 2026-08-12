// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const PERIOD = 30;

/**
 * @template T
 * @param {(dir: string) => T} fn
 * @returns {T}
 */
function withTmp(fn) {
  const dir = mkdtempSync(join(tmpdir(), 'testcard-'));
  try { return fn(dir); } finally { rmSync(dir, { recursive: true, force: true }); }
}

/**
 * Extract a single frame as raw gray bytes.
 * @param {string} file
 * @param {number} index
 * @returns {Buffer}
 */
function frameBytes(file, index) {
  return execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', file,
    '-vf', `select=eq(n\\,${index})`, '-fps_mode', 'passthrough', '-frames:v', '1',
    '-f', 'rawvideo', '-pix_fmt', 'gray', '-'], { maxBuffer: 64 * 1024 * 1024 });
}

test('frame P is bit-identical to frame 0 — the wrap is matched by construction', () => {
  withTmp((dir) => {
    const out = join(dir, 'card.mkv');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', String(PERIOD + 1), '--period', String(PERIOD), '--output', out]);
    const first = frameBytes(out, 0);
    const wrap = frameBytes(out, PERIOD);
    assert.equal(Buffer.compare(first, wrap), 0, 'frame P must equal frame 0 exactly');
  });
});

test('consecutive frames differ — the pattern actually moves', () => {
  withTmp((dir) => {
    const out = join(dir, 'card.mkv');
    execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
      '--fps', '30', '--frames', String(PERIOD), '--output', out]);
    assert.notEqual(Buffer.compare(frameBytes(out, 0), frameBytes(out, 1)), 0);
  });
});

test('renders exactly the requested frame count at the requested size', () => {
  withTmp((dir) => {
    const out = join(dir, 'card.mkv');
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
