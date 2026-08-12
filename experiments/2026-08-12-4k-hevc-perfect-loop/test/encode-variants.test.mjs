// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/**
 * @param {string} file
 * @param {string[]} [extra]
 * @returns {any}
 */
function ffprobeJson(file, extra = []) {
  return JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-of', 'json', ...extra, file]).toString());
}

/**
 * @param {string} dir
 * @returns {string}
 */
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
    const video = streams.find((/** @type {any} */ s) => s.codec_type === 'video');
    assert.equal(video.codec_name, 'hevc');
    assert.equal(streams.some((/** @type {any} */ s) => s.codec_type === 'audio'), false, 'must be silent');

    const frames = ffprobeJson(mp4, ['-select_streams', 'v:0', '-show_frames',
      '-show_entries', 'frame=key_frame']).frames;
    assert.equal(Number(frames[0].key_frame), 1, 'frame 0 must be a keyframe');
    const keyframes = frames.filter((/** @type {any} */ f) => Number(f.key_frame) === 1).length;
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
