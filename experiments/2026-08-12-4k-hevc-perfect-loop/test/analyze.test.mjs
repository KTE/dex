// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { analyze } from '../lib/analyze.mjs';

const N = 30;

/**
 * Perfect capture: `wraps` clean loops of length N.
 * @param {number} wraps
 * @returns {(number|null)[]}
 */
function perfect(wraps) {
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < wraps; w++) for (let i = 0; i < N; i++) out.push(i);
  return out;
}

/**
 * Deterministic LCG so the noise test is reproducible.
 * @param {number} seed
 * @returns {() => number}
 */
function lcg(seed) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

test('a perfect capture passes', () => {
  const r = analyze(perfect(600), { loopLength: N });
  assert.equal(r.verdict, 'PASS');
  assert.equal(r.wrapAnomalies, 0);
  assert.equal(r.midAnomalies, 0);
  assert.ok(r.wrapTransitions >= 500);
});

test('a held frame at every wrap fails', () => {
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) out.push(i);
    out.push(N - 1); // last frame held one extra capture frame
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
  assert.ok(r.wrapRate > r.midRate);
});

test('a dropped frame at every wrap fails', () => {
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) for (let i = 0; i < N - 1; i++) out.push(i); // N-1 never shown
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
});

test('a black frame at every wrap fails and is counted as a decode failure', () => {
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) out.push(i);
    out.push(null);
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
  assert.equal(r.decodeFailures, 1199, 'each null spans two transitions; the final null has no successor');
});

test('CONTROL: uniform capture noise passes — it is the noise floor, not a seam', () => {
  // 1% of frames duplicated at uniformly random positions. This is the control
  // that makes the whole method valid: if this failed, every run would fail.
  const rnd = lcg(12345);
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) {
      out.push(i);
      if (rnd() < 0.01) out.push(i);
    }
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'PASS', `noise must not read as a seam (p=${r.p})`);
});

test('anomalies concentrated mid-loop void the run rather than passing it', () => {
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) {
      out.push(i);
      if (i === 10) out.push(i); // deterministic mid-loop stutter, never at the wrap
    }
  }
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'VOID');
});

test('too few wraps is INSUFFICIENT, not PASS', () => {
  const r = analyze(perfect(100), { loopLength: N });
  assert.equal(r.verdict, 'INSUFFICIENT');
});

test('against a perfectly clean floor, even one wrap defect is significant', () => {
  // Not the intuition the spec was first written with, but the correct
  // behaviour: the test compares against the MEASURED floor, and when that
  // floor is exactly zero, a single wrap anomaly is real signal (z=5.4).
  // At a 1s loop that is a visible stutter every ~10 minutes for six weeks.
  const out = perfect(600);
  out.splice(N * 300, 0, N - 1); // one held frame, once
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'FAIL');
});

test('CONTROL: the same single defect passes once a realistic noise floor exists', () => {
  // The counterpart to the test above — this is the control doing its job.
  // With ~1% uniform capture noise present, one extra defect at a wrap is
  // genuinely indistinguishable from that noise, and must not fail the run.
  const rnd = lcg(999);
  /** @type {(number|null)[]} */
  const out = [];
  for (let w = 0; w < 600; w++) {
    for (let i = 0; i < N; i++) {
      out.push(i);
      if (rnd() < 0.01) out.push(i);
    }
  }
  out.splice(N * 300, 0, N - 1); // the same single planted defect
  const r = analyze(out, { loopLength: N });
  assert.equal(r.verdict, 'PASS', `one defect must vanish into a real noise floor (p=${r.p})`);
});

test('loopLength is inferred from the data when not supplied', () => {
  const r = analyze(perfect(600));
  assert.equal(r.loopLength, N);
});

test('an all-null capture is rejected rather than analysed', () => {
  assert.throws(() => analyze([null, null, null]), /no decodable frames/);
});
