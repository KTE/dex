#!/usr/bin/env node
// @ts-check
import { readFileSync } from 'node:fs';
import { analyze } from '../lib/analyze.mjs';

const args = process.argv.slice(2);
/** @type {(flag: string) => string | undefined} */
const get = (flag) => {
  const i = args.indexOf(flag);
  return i === -1 ? undefined : args[i + 1];
};

const file = get('--log');
if (!file) {
  console.error('usage: analyze.mjs --log <indices.txt> [--loop-length N] [--alpha 0.01] [--min-wraps 500] [--json]');
  process.exit(2);
}

/** @type {(number|null)[]} */
const indices = readFileSync(file, 'utf8').split('\n')
  .filter((l) => l.trim() !== '')
  .map((l) => (l.trim() === 'null' ? null : Number(l)));

const loopLengthArg = get('--loop-length');
const alphaArg = get('--alpha');
const minWrapsArg = get('--min-wraps');

const result = analyze(indices, {
  loopLength: loopLengthArg ? Number(loopLengthArg) : undefined,
  alpha: alphaArg ? Number(alphaArg) : undefined,
  minWraps: minWrapsArg ? Number(minWrapsArg) : undefined,
});

if (args.includes('--json')) {
  console.log(JSON.stringify(result, null, 2));
} else {
  console.log(`verdict:      ${result.verdict}`);
  console.log(`reason:       ${result.reason}`);
  console.log(`loop length:  ${result.loopLength} frames`);
  console.log(`wraps:        ${result.wrapAnomalies}/${result.wrapTransitions} anomalous`);
  console.log(`mid-loop:     ${result.midAnomalies}/${result.midTransitions} anomalous (noise floor)`);
  console.log(`decode fails: ${result.decodeFailures}`);
}

// Exit non-zero on anything that is not a clean pass, so shell pipelines and
// the soak runner can branch on it without parsing stdout.
process.exit(result.verdict === 'PASS' ? 0 : 1);
