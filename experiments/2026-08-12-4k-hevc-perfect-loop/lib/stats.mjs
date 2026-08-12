// @ts-check

/**
 * Error function, Abramowitz & Stegun 7.1.26.
 * Max absolute error 1.5e-7 — orders of magnitude tighter than an alpha=0.01 gate needs.
 * @param {number} x
 * @returns {number}
 */
export function erf(x) {
  const sign = x < 0 ? -1 : 1;
  const ax = Math.abs(x);
  const t = 1 / (1 + 0.3275911 * ax);
  const poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
  return sign * (1 - poly * Math.exp(-ax * ax));
}

/**
 * Standard normal cumulative distribution function.
 * @param {number} z
 * @returns {number}
 */
export function normalCdf(z) {
  return 0.5 * (1 + erf(z / Math.SQRT2));
}

/**
 * Two-proportion z-test, two-sided.
 *
 * Group 1 is the wrap-transition sample, group 2 the mid-loop sample. The null
 * hypothesis is that both come from the same underlying anomaly rate — i.e. that
 * everything observed at the wrap is explained by capture noise.
 *
 * @param {{x1: number, n1: number, x2: number, n2: number}} input
 * @returns {{p1: number, p2: number, z: number, p: number}}
 */
export function twoProportionTest({ x1, n1, x2, n2 }) {
  if (n1 <= 0 || n2 <= 0) throw new Error('n1 and n2 must be positive');
  const p1 = x1 / n1;
  const p2 = x2 / n2;
  const pooled = (x1 + x2) / (n1 + n2);
  const se = Math.sqrt(pooled * (1 - pooled) * (1 / n1 + 1 / n2));
  // Degenerate case: no anomalies anywhere (or all anomalies everywhere) means
  // there is no variance to test against. That is "no detectable difference",
  // not an error — and emphatically not a pass-by-division-by-zero.
  if (se === 0) return { p1, p2, z: 0, p: 1 };
  const z = (p1 - p2) / se;
  const p = 2 * (1 - normalCdf(Math.abs(z)));
  return { p1, p2, z, p };
}
