// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const LOOP = 30;
const WRAPS = 600;

/**
 * Run the real pipeline — generate, burn, capture — and return the frame-index
 * sequence for one pass of the loop.
 * @param {string} dir
 * @returns {string[]}
 */
function buildLoopedCapture(dir) {
  const testCard = join(dir, 'card.mkv');
  const burned = join(dir, 'burned.mkv');
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
    const seamed = Array.from({ length: WRAPS },
      () => [...one, one[one.length - 1]].join('\n')).join('\n');
    const full = join(dir, 'seam.txt');
    writeFileSync(full, seamed + '\n');

    let code = 0;
    let out = '';
    try {
      out = execFileSync('node', ['bin/analyze.mjs', '--log', full,
        '--loop-length', String(LOOP)]).toString();
    } catch (e) {
      const err = /** @type {any} */ (e);
      code = err.status;
      out = err.stdout.toString();
    }
    assert.equal(code, 1, 'analyze must exit non-zero on a seam');
    assert.match(out, /verdict:\s+FAIL/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('E2E: a black frame at the wrap is caught as a decode failure, not a wrong index', () => {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-'));
  try {
    const one = buildLoopedCapture(dir);
    const blacked = Array.from({ length: WRAPS },
      () => [...one, 'null'].join('\n')).join('\n');
    const full = join(dir, 'black.txt');
    writeFileSync(full, blacked + '\n');

    let out = '';
    try {
      out = execFileSync('node', ['bin/analyze.mjs', '--log', full,
        '--loop-length', String(LOOP), '--json']).toString();
    } catch (e) {
      out = /** @type {any} */ (e).stdout.toString();
    }
    const r = JSON.parse(out);
    assert.equal(r.verdict, 'FAIL');
    assert.ok(r.decodeFailures > 0, 'black frames must surface as decode failures');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
