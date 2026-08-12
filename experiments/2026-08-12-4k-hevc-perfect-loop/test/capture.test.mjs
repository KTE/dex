// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/**
 * @param {string} dir
 * @param {number} frames
 * @returns {string} path to the barcoded clip
 */
function buildBurned(dir, frames) {
  const testCard = join(dir, 'card.mkv');
  const burned = join(dir, 'burned.mkv');
  execFileSync('bash', ['scripts/make-test-card.sh', '--width', '320', '--height', '192',
    '--fps', '30', '--frames', String(frames), '--output', testCard]);
  execFileSync('bash', ['scripts/add-barcode.sh', '--input', testCard, '--output', burned]);
  return burned;
}

test('capture from a file yields the exact frame-index sequence', () => {
  const dir = mkdtempSync(join(tmpdir(), 'cap-'));
  try {
    const burned = buildBurned(dir, 45);
    const log = join(dir, 'idx.txt');
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
    const burned = buildBurned(dir, 45);
    const log = join(dir, 'idx.txt');
    execFileSync('node', ['bin/capture.mjs', '--source', burned, '--height', '192',
      '--out', log, '--frames', '10']);
    assert.equal(readFileSync(log, 'utf8').trim().split('\n').length, 10);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
