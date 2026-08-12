// @ts-check
import { twoProportionTest } from './stats.mjs';

/**
 * @typedef {Object} AnalyzeResult
 * @property {number} loopLength
 * @property {number} wrapTransitions
 * @property {number} wrapAnomalies
 * @property {number} midTransitions
 * @property {number} midAnomalies
 * @property {number} wrapRate
 * @property {number} midRate
 * @property {number} z
 * @property {number} p
 * @property {number} decodeFailures
 * @property {'PASS'|'FAIL'|'VOID'|'INSUFFICIENT'} verdict
 * @property {string} reason
 */

/**
 * Classify every transition in a captured frame-index stream and decide whether
 * the wrap point behaves differently from the rest of the loop.
 *
 * The comparison — not the absolute anomaly count — is the measurement. USB
 * capture is not frame-locked to the player, so it drops and duplicates frames
 * on its own; those artifacts are uniformly distributed while a real seam is
 * concentrated at the wrap. The mid-loop rate is therefore the noise floor,
 * measured on the same run by the same instrument.
 *
 * Transition classification, walking consecutive pairs (prev, cur):
 *
 *   prev or cur is null   -> wrap if lastGood === N-1 else mid; anomalous
 *   cur === prev + 1      -> mid;  clean
 *   prev === N-1, cur = 0 -> wrap; clean (this is a good wrap)
 *   cur < prev            -> wrap; anomalous (a reset that is not N-1 -> 0)
 *   prev === N-1          -> wrap; anomalous (held or skipped at the boundary)
 *   otherwise             -> mid;  anomalous (repeat or skip mid-loop)
 *
 * @param {(number|null)[]} indices - decoded frame index per captured frame
 * @param {{loopLength?: number, alpha?: number, minWraps?: number}} [opts]
 * @returns {AnalyzeResult}
 */
export function analyze(indices, opts = {}) {
  const alpha = opts.alpha ?? 0.01;
  const minWraps = opts.minWraps ?? 500;

  /** @type {number[]} */
  const good = /** @type {number[]} */ (indices.filter((v) => v !== null));
  if (good.length === 0) throw new Error('no decodable frames');
  const loopLength = opts.loopLength ?? Math.max(...good) + 1;
  const last = loopLength - 1;

  let wrapTransitions = 0, wrapAnomalies = 0;
  let midTransitions = 0, midAnomalies = 0, decodeFailures = 0;
  /** @type {number|null} */
  let lastGood = null;

  for (let i = 0; i + 1 < indices.length; i++) {
    const prev = indices[i];
    const cur = indices[i + 1];
    if (prev !== null) lastGood = prev;

    /** @type {'wrap'|'mid'} */
    let bucket;
    let anomalous;

    if (prev === null || cur === null) {
      decodeFailures++;
      bucket = lastGood === last ? 'wrap' : 'mid';
      anomalous = true;
    } else if (cur === prev + 1) {
      bucket = 'mid'; anomalous = false;
    } else if (prev === last && cur === 0) {
      bucket = 'wrap'; anomalous = false;
    } else if (cur < prev || prev === last) {
      bucket = 'wrap'; anomalous = true;
    } else {
      bucket = 'mid'; anomalous = true;
    }

    if (bucket === 'wrap') { wrapTransitions++; if (anomalous) wrapAnomalies++; }
    else { midTransitions++; if (anomalous) midAnomalies++; }
  }

  const wrapRate = wrapTransitions ? wrapAnomalies / wrapTransitions : 0;
  const midRate = midTransitions ? midAnomalies / midTransitions : 0;

  if (wrapTransitions < minWraps) {
    return {
      loopLength, wrapTransitions, wrapAnomalies, midTransitions, midAnomalies,
      wrapRate, midRate, z: 0, p: 1, decodeFailures,
      verdict: 'INSUFFICIENT',
      reason: `only ${wrapTransitions} wrap transitions; need >= ${minWraps}`,
    };
  }

  const { z, p } = twoProportionTest({
    x1: wrapAnomalies, n1: wrapTransitions,
    x2: midAnomalies, n2: midTransitions,
  });

  /** @type {'PASS'|'FAIL'|'VOID'} */
  let verdict;
  let reason;
  if (p >= alpha) {
    verdict = 'PASS';
    reason = `wrap rate ${wrapRate.toFixed(5)} indistinguishable from noise floor ${midRate.toFixed(5)} (p=${p.toFixed(4)})`;
  } else if (wrapRate > midRate) {
    verdict = 'FAIL';
    reason = `wrap rate ${wrapRate.toFixed(5)} exceeds noise floor ${midRate.toFixed(5)} (p=${p.toExponential(2)}) — seam detected`;
  } else {
    verdict = 'VOID';
    reason = `wrap rate ${wrapRate.toFixed(5)} is BELOW noise floor ${midRate.toFixed(5)} (p=${p.toExponential(2)}) — wraps are being misclassified, check loopLength`;
  }

  return {
    loopLength, wrapTransitions, wrapAnomalies, midTransitions, midAnomalies,
    wrapRate, midRate, z, p, decodeFailures, verdict, reason,
  };
}
