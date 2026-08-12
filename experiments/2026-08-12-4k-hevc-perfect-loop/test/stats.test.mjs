// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { erf, normalCdf, twoProportionTest } from '../lib/stats.mjs';

test('erf(0) is 0 and erf saturates to 1', () => {
  // A&S 7.1.26 is an approximation with a documented max error of 1.5e-7; it
  // leaves ~1e-9 residual at the origin. Asserting exact zero would be testing
  // a property the algorithm never claimed.
  assert.ok(Math.abs(erf(0)) < 1e-7);
  assert.ok(Math.abs(erf(3) - 0.9999779) < 1e-5);
  assert.ok(Math.abs(erf(-3) + 0.9999779) < 1e-5);
});

test('normalCdf matches known quantiles', () => {
  assert.ok(Math.abs(normalCdf(0) - 0.5) < 1e-9);
  assert.ok(Math.abs(normalCdf(1.959964) - 0.975) < 1e-4);
  assert.ok(Math.abs(normalCdf(-2.575829) - 0.005) < 1e-4);
});

test('identical proportions give p near 1', () => {
  const r = twoProportionTest({ x1: 50, n1: 1000, x2: 50, n2: 1000 });
  assert.equal(r.z, 0);
  assert.ok(r.p > 0.99);
});

test('a defect at every wrap is detected', () => {
  // 500 wraps, every one anomalous; mid-loop clean apart from noise.
  const r = twoProportionTest({ x1: 500, n1: 500, x2: 10, n2: 14500 });
  assert.ok(r.p1 > r.p2);
  assert.ok(r.p < 1e-6, `expected tiny p, got ${r.p}`);
});

test('zero anomalies on both sides is not a difference', () => {
  const r = twoProportionTest({ x1: 0, n1: 500, x2: 0, n2: 14500 });
  assert.equal(r.z, 0);
  assert.equal(r.p, 1);
});

test('empty sample is rejected rather than silently returning NaN', () => {
  assert.throws(() => twoProportionTest({ x1: 0, n1: 0, x2: 1, n2: 10 }), /n1 and n2 must be positive/);
});
