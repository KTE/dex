// @ts-check
// SPDX-License-Identifier: MIT-0
/**
 * Tests for docs-lint. Run with:  node --test scripts/docs-lint.test.mjs
 *
 * Every rule gets a positive fixture and the false-positive classes named in
 * the lint spec, so a future change to a regex has to survive them.
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  run, lintSource, makeCtx, parseAllowlist, parseGlossary, parseCoinages, ConfigError,
} from './docs-lint.mjs';

/**
 * @param {string} p
 * @param {string} text
 * @param {Record<string, unknown>} [ctxOver]
 */
function lint(p, text, ctxOver = {}) {
  return lintSource(p, text, makeCtx(ctxOver));
}

/** @param {ReturnType<typeof lint>} findings @param {string} rule */
function of(findings, rule) {
  return findings.filter((f) => f.rule === rule);
}

/** @param {ReturnType<typeof lint>} findings @param {string} rule */
function tokens(findings, rule) {
  return of(findings, rule).map((f) => f.token);
}

// ---------------------------------------------------------------------------
// plan-codes
// ---------------------------------------------------------------------------

test('plan-codes: flags F6 in a doc comment and in an identifier', () => {
  const src = [
    '//! F6 — the exhibit display config.',
    'fn f6_missing_config() {}',
  ].join('\n');
  const f = of(lint('src/exhibit.rs', src), 'plan-codes');
  assert.deepEqual(f.map((x) => [x.line, x.token]), [[1, 'F6'], [2, 'f6']]);
  assert.equal(f[0].severity, 'E');
});

test('plan-codes: i64::MAX and other code are not prose, CM4 and M.2 do not match', () => {
  const src = [
    'let x = i64::MAX;                 // a CM4 with an M.2 SSD',
    '/// Pi 4 and CM4 boards, M.2 carrier',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/chunk.rs', src), 'plan-codes'), []);
});

test('plan-codes: markdown code spans and fences are stripped first', () => {
  const src = [
    'The `F6` config and:',
    '',
    '```',
    'F6 T7 M5',
    '```',
    '',
    'but F6 in prose is a hit.',
  ].join('\n');
  assert.deepEqual(tokens(lint('docs/x.md', src), 'plan-codes'), ['F6']);
});

// ---------------------------------------------------------------------------
// private-refs
// ---------------------------------------------------------------------------

test('private-refs: PLAN.md is an error, "the design" only a warning', () => {
  const src = '# see PLAN.md F10 for the bench unit; the design says otherwise, §5c\n';
  const f = of(lint('deploy/dexd.service', src), 'private-refs');
  const bySeverity = Object.fromEntries(f.map((x) => [x.token.trim(), x.severity]));
  assert.equal(bySeverity['PLAN.md'], 'E');
  assert.equal(bySeverity['the design'], 'W');
  assert.ok(f.some((x) => x.token.startsWith('§')));
});

// ---------------------------------------------------------------------------
// dates
// ---------------------------------------------------------------------------

test('dates: heading date, date-as-justification, plain date, time-of-day split', () => {
  const src = [
    '## Reviewed 2026-08-15',
    '',
    'No asset path, as of 2026-08-17: the config names the artwork.',
    '',
    'Recorded 2026-08-12 on the record.',
    '',
    '### 2026-08-15 (later) — the rule was withdrawn',
  ].join('\n');
  const f = of(lint('docs/x.md', src), 'dates');
  const kinds = f.map((x) => `${x.line}:${x.severity}`);
  assert.ok(kinds.includes('1:E'), 'heading date is an error');
  assert.ok(kinds.includes('3:E'), 'date as justification is an error');
  assert.ok(kinds.includes('5:W'), 'a plain ISO date is a warning');
  assert.ok(f.some((x) => x.token === '(later)' && x.severity === 'E'));
});

test('dates: a .TH line in a man page is the roff contract, not prose', () => {
  const src = [
    '.TH DEXD 1 "2026-08-17" "dexd 0.1.0" "dex"',
    '.SH NAME',
    'dexd \\- gapless HEVC loop player',
  ].join('\n');
  assert.deepEqual(of(lint('deploy/man/dexd.1', src), 'dates'), []);
});

test('dates: a version string date in Cargo.toml is not comment prose', () => {
  const src = 'version = "0.1.0-2026-08-17.abc"\n';
  assert.deepEqual(of(lint('Cargo.toml', src), 'dates'), []);
});

test('dates: a changelog is dated by definition', () => {
  const src = 'dexd (0.1.0-2) unstable; urgency=medium\n -- max <m@x>  Sun, 2026-08-17\n';
  assert.deepEqual(of(lint('deploy/changelog', src), 'dates'), []);
});

// ---------------------------------------------------------------------------
// caps
// ---------------------------------------------------------------------------

test('caps: dictionary-word emphasis versus unknown acronym', () => {
  const src = '//! The mode is a property of the INSTALLATION, and MUST use QZXW.\n';
  const ctx = { dict: new Set(['installation', 'must']) };
  const f = of(lint('src/exhibit.rs', src), 'caps');
  const byToken = Object.fromEntries(
    lintSource('src/exhibit.rs', src, makeCtx(ctx)).filter((x) => x.rule === 'caps')
      .map((x) => [x.token, x.message]));
  assert.match(byToken.INSTALLATION, /dictionary-word emphasis/);
  assert.match(byToken.QZXW, /unknown acronym/);
  assert.ok(f.every((x) => x.severity === 'E'));
});

test('caps: SCREAMING_CASE, HDMI-A-1, READY=1 and glossary acronyms are not emphasis', () => {
  const src = [
    '//! MPV_EVENT_END_FILE arrives on HDMI-A-1 once READY=1 is sent.',
    '//! HEVC and EDID are glossary terms. SAFETY: the pointer is owned.',
  ].join('\n');
  const ctx = { acronyms: new Set(['HEVC', 'EDID']) };
  assert.deepEqual(tokens(lintSource('src/main.rs', src, makeCtx(ctx)), 'caps'), []);
});

test('caps: man-page section names are allowed inside .1 files only', () => {
  const man = '.SH DESCRIPTION\nThe SYNOPSIS above is the contract.\n';
  assert.deepEqual(tokens(lint('deploy/man/dexd.1', man), 'caps'), []);
  assert.deepEqual(tokens(lint('docs/x.md', 'The SYNOPSIS above is the contract.\n'), 'caps'),
    ['SYNOPSIS']);
});

// ---------------------------------------------------------------------------
// bold
// ---------------------------------------------------------------------------

test('bold: a bolded negation is an error; density is a warning', () => {
  const body = 'filler word '.repeat(40);
  const src = `The override is **not** applied.\n\n${body}\n`;
  const f = of(lint('docs/x.md', src), 'bold');
  assert.equal(f[0].severity, 'E');
  assert.equal(f[0].token, '**not**');
});

test('bold: a bold path or literal is not counted', () => {
  const src = '**/etc/dex/exhibit.json** is the conffile. ' + 'word '.repeat(400) + '\n';
  assert.deepEqual(of(lint('docs/x.md', src), 'bold'), []);
});

// ---------------------------------------------------------------------------
// intensifiers
// ---------------------------------------------------------------------------

test('intensifiers: the house words are errors', () => {
  const src = '// 2>&1 is load-bearing, and NO PATH FILTERS, DELIBERATELY: that is the point.\n';
  const t = tokens(lint('src/main.rs', src), 'intensifiers').map((s) => s.toLowerCase());
  assert.ok(t.includes('load-bearing'));
  assert.ok(t.includes('deliberately'));
  assert.ok(t.includes('that is the point'));
});

test('intensifiers: "exactly 90 frames" is legitimate and never reported', () => {
  const src = '/// The chunk is exactly 90 frames long, exactly one GOP, exactly the same.\n';
  assert.deepEqual(tokens(lint('src/chunk.rs', src), 'intensifiers'), []);
});

// ---------------------------------------------------------------------------
// contrast
// ---------------------------------------------------------------------------

test('contrast: warns only above the density threshold, and caps the report at 10', () => {
  const line = 'It is venue truth, not asset truth, rather than a guess.\n';
  const f = of(lint('docs/x.md', line.repeat(12)), 'contrast');
  assert.ok(f.length > 0 && f.length <= 10);
  assert.ok(f.every((x) => x.severity === 'W'));
  assert.match(f[0].message, /per 1000 words/);
});

test('contrast: a single contrast in a long file is under the threshold', () => {
  const src = 'word '.repeat(2000) + '\nvenue truth, not asset truth\n';
  assert.deepEqual(of(lint('docs/x.md', src), 'contrast'), []);
});

// ---------------------------------------------------------------------------
// coinages
// ---------------------------------------------------------------------------

test('coinages: soak, the wrap and bare seam are errors that name the replacement', () => {
  const src = [
    '//! The 24 h soak ran at the wrap; the seam was visible.',
    'fn seam_detected() {}',
  ].join('\n');
  const f = of(lint('src/main.rs', src), 'coinages');
  const t = f.map((x) => x.token.toLowerCase());
  assert.ok(t.includes('soak'));
  assert.ok(t.includes('at the wrap'));
  assert.ok(t.includes('seam'));
  assert.ok(f.some((x) => /loop point/.test(x.message)), 'the message prints the replacement');
});

test('coinages: "seamless" is the approved word and "wraps back to byte 0" is mechanism', () => {
  const src = '/// Playback is seamless; the offset wraps back to byte 0 at the end.\n';
  assert.deepEqual(tokens(lint('src/chunk.rs', src), 'coinages'), []);
});

test('coinages: the table can be extended from a file', () => {
  const extra = parseCoinages('flanging\tringing artefact\n# comment\n');
  assert.deepEqual(extra.map((c) => c.key), ['flanging']);
  const f = of(lint('docs/x.md', 'The flanging is visible.\n', { coinages: extra }), 'coinages');
  assert.equal(f.length, 1);
  assert.match(f[0].message, /ringing artefact/);
});

// ---------------------------------------------------------------------------
// codenames
// ---------------------------------------------------------------------------

test('codenames: bench hosts, session referents and card IDs', () => {
  const src = [
    '/// Verified on dexpi4 tonight; the bench Pi ran card e04.',
  ].join('\n');
  const t = tokens(lint('src/main.rs', src), 'codenames').map((s) => s.toLowerCase());
  assert.ok(t.includes('dexpi4'));
  assert.ok(t.includes('tonight'));
  assert.ok(t.includes('the bench pi'));
  assert.ok(t.includes('e04'));
});

test('codenames: card IDs are allowed in the measurement record', () => {
  const src = '| e04 | 2160p30 | pass |\n';
  assert.deepEqual(tokens(lint('docs/design/measurements.md', src), 'codenames'), []);
});

test('codenames: "the Cam Link" is fine after the product is introduced, not before', () => {
  const introduced = 'An Elgato Cam Link 4K HDMI capture card is the sink.\nSo the Cam Link forces the mode.\n';
  assert.deepEqual(tokens(lint('docs/x.md', introduced), 'codenames'), []);
  const bare = 'So the Cam Link forces the mode.\n';
  assert.deepEqual(tokens(lint('docs/y.md', bare), 'codenames'), ['the Cam Link']);
});

// ---------------------------------------------------------------------------
// first-person
// ---------------------------------------------------------------------------

test('first-person: pronouns in prose are errors', () => {
  const src = '/// We measured 0.753x realtime, and our margin is 17x.\n';
  const t = tokens(lint('src/health.rs', src), 'first-person');
  assert.deepEqual(t, ['We', 'our']);
});

test('first-person: I/O is skipped, but a pronoun inside a quotation is reported (§4.9)', () => {
  const io = '/// I/O errors are surfaced verbatim.\n';
  assert.deepEqual(tokens(lint('src/health.rs', io), 'first-person'), []);

  // §4.9: "quotes are not stripped by default; report and let the allowlist
  // take individual lines". A quoted pronoun still ships to the reader.
  const quoted = '/// As the kernel doc puts it, "we cannot know the sink here".\n';
  assert.deepEqual(tokens(lint('src/health.rs', quoted), 'first-person'), ['we']);
  const allow = parseAllowlist('first-person path=src/health.rs we # quoted from the kernel docs\n');
  assert.deepEqual(tokens(lint('src/health.rs', quoted, { allow }), 'first-person'), []);
});

// ---------------------------------------------------------------------------
// process-talk
// ---------------------------------------------------------------------------

test('process-talk: review and revision talk', () => {
  const src = [
    '/// bench-confirmed live, three independent reviews.',
    '/// CORRECTION: an earlier draft claimed otherwise; this used to live in the unit.',
  ].join('\n');
  const t = tokens(lint('src/main.rs', src), 'process-talk').map((s) => s.toLowerCase());
  assert.ok(t.includes('three independent reviews'));
  assert.ok(t.includes('correction'));
  assert.ok(t.includes('an earlier draft'));
  assert.ok(t.includes('used to live'));
});

test('process-talk: "used to compute" is purpose, not history', () => {
  const src = '/// The nonce is used to compute the digest, and used to seed the hash.\n';
  assert.deepEqual(tokens(lint('src/sha256.rs', src), 'process-talk'), []);
});

test('process-talk: MAJOR/MINOR are only triage words in .rs and .md', () => {
  const rs = '/// MINOR: the second reader is redundant.\n';
  assert.deepEqual(tokens(lint('src/main.rs', rs), 'process-talk'), ['MINOR']);
  const sh = '# MAJOR version bump handling\n';
  assert.deepEqual(tokens(lint('deploy/maintainer-scripts/postinst', sh), 'process-talk'), []);
});

// ---------------------------------------------------------------------------
// comment-length
// ---------------------------------------------------------------------------

test('comment-length: 20 lines warn, 40 lines error, argument markers warn', () => {
  const short = Array.from({ length: 25 }, (_, i) => `//! line ${i}`).join('\n') + '\nfn a() {}\n';
  const long = Array.from({ length: 45 }, (_, i) => `//! line ${i}`).join('\n') + '\nfn a() {}\n';
  assert.equal(of(lint('src/a.rs', short), 'comment-length')[0].severity, 'W');
  assert.equal(of(lint('src/a.rs', long), 'comment-length')[0].severity, 'E');

  const arguing = [
    '/// Why this exists.',
    '/// It is here because the alternative was proposed and rejected.',
    'fn a() {}',
  ].join('\n');
  const f = of(lint('src/a.rs', arguing), 'comment-length');
  assert.ok(f.some((x) => /argument markers/.test(x.message)));
});

// ---------------------------------------------------------------------------
// structure
// ---------------------------------------------------------------------------

test('structure: emoji headings, emoji bullets, session-verb headings', () => {
  const src = [
    '## ✅ Done',
    '',
    '- ✅ Experiment framed',
    '',
    '## Reviewed the failure paths',
  ].join('\n');
  const f = of(lint('docs/x.md', src), 'structure');
  assert.equal(f[0].severity, 'E');
  assert.equal(f[1].severity, 'W');
  assert.equal(f[2].token, 'Reviewed');
  assert.equal(f[2].severity, 'E');
});

test('structure: a "#" comment in a shell script is not a heading', () => {
  assert.deepEqual(of(lint('deploy/dex-wait-hdmi', '#!/bin/sh\n# Verified on hardware\n'), 'structure'), []);
});

// ---------------------------------------------------------------------------
// allowlist parser
// ---------------------------------------------------------------------------

test('allowlist: every entry needs a reason', () => {
  assert.throws(() => parseAllowlist('caps SAND\n'), ConfigError);
  assert.throws(() => parseAllowlist('caps SAND #\n'), ConfigError);
  assert.throws(() => parseAllowlist('no-such-rule X # reason\n'), ConfigError);
  const ok = parseAllowlist('# a comment\n\ncaps SAND # Broadcom tile format\n');
  assert.equal(ok.length, 1);
  assert.deepEqual(
    { rule: ok[0].rule, token: ok[0].tokenLiteral, glob: ok[0].pathGlob },
    { rule: 'caps', token: 'SAND', glob: null });
});

test('allowlist: global, scoped and regex entries filter findings', () => {
  const src = '//! The QZXW register is set.\n';
  assert.equal(of(lint('src/a.rs', src), 'caps').length, 1);

  const global = parseAllowlist('caps QZXW # a real register name\n');
  assert.equal(of(lint('src/a.rs', src, { allow: global }), 'caps').length, 0);

  const scoped = parseAllowlist('caps path=**/*.rs QZXW # only in Rust\n');
  assert.equal(of(lint('src/a.rs', src, { allow: scoped }), 'caps').length, 0);
  assert.equal(of(lint('docs/a.md', 'The QZXW register.\n', { allow: scoped }), 'caps').length, 1);

  const rx = parseAllowlist('caps /^Q/ # anything starting with Q\n');
  assert.equal(of(lint('src/a.rs', src, { allow: rx }), 'caps').length, 0);
});

// ---------------------------------------------------------------------------
// glossary parser
// ---------------------------------------------------------------------------

test('glossary: headings and "Also written" aliases become the acronym set', () => {
  const g = [
    '# Glossary',
    '',
    '### HEVC',
    'Also written: H.265, HEV1',
    '',
    'The video codec.',
    '',
    '### loop point',
    'Where playback returns to the first frame.',
    '',
    '### EDID',
    '',
  ].join('\n');
  const { acronyms, terms } = parseGlossary(g);
  assert.ok(acronyms.has('HEVC'));
  assert.ok(acronyms.has('HEV1'));
  assert.ok(acronyms.has('EDID'));
  assert.ok(!acronyms.has('loop point'));
  assert.ok(terms.has('loop point'));
});

test('glossary: a term in the glossary stops being an unknown acronym', () => {
  const src = '//! The EDID is read at start.\n';
  assert.equal(of(lint('src/a.rs', src), 'caps').length, 1);
  const { acronyms } = parseGlossary('### EDID\nThe display descriptor.\n');
  assert.equal(of(lint('src/a.rs', src, { acronyms }), 'caps').length, 0);
});

// ---------------------------------------------------------------------------
// end to end
// ---------------------------------------------------------------------------

test('end to end: runs over a directory, honours --rules, --format json and --warn-only', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs-lint-'));
  fs.mkdirSync(path.join(dir, 'docs'), { recursive: true });
  fs.mkdirSync(path.join(dir, 'src'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'src', 'main.rs'), '//! F6 — the entry point.\nfn main() {}\n');
  fs.writeFileSync(path.join(dir, 'docs', 'guide.md'), '# Guide\n\nRun the 24 h soak on dexpi4.\n');
  fs.writeFileSync(path.join(dir, 'docs', 'glossary.md'), '### HEVC\nThe codec.\n');
  fs.writeFileSync(path.join(dir, 'docs', 'lint-allow.txt'), 'caps SAND # Broadcom tile format\n');

  /** @type {string[]} */
  let out = [];
  const io = { log: (s) => out.push(s), err: () => {}, cwd: dir };

  let code = run(['src', 'docs'], io);
  assert.equal(code, 1, 'errors exit 1');
  const text = out.join('\n');
  assert.match(text, /src\/main\.rs:1:5 \[E\] plan-codes/);
  assert.match(text, /docs\/guide\.md:3:\d+ \[E\] coinages/);
  assert.match(text, /docs-lint summary/);

  out = [];
  code = run(['--warn-only', 'src', 'docs'], io);
  assert.equal(code, 0, '--warn-only exits 0');

  out = [];
  code = run(['--format', 'json', '--rules', 'plan-codes', 'src', 'docs'], io);
  const json = JSON.parse(out.join('\n'));
  assert.ok(Array.isArray(json));
  assert.ok(json.length >= 1);
  assert.ok(json.every((x) => x.rule === 'plan-codes'));
  assert.deepEqual(Object.keys(json[0]).sort(),
    ['col', 'line', 'message', 'path', 'rule', 'severity', 'token']);

  out = [];
  assert.equal(run(['--rules', 'no-such-rule', 'src'], io), 2, 'bad rule id exits 2');

  fs.writeFileSync(path.join(dir, 'docs', 'lint-allow.txt'), 'caps SAND\n');
  assert.equal(run(['src'], io), 2, 'allowlist entry without a reason exits 2');

  fs.rmSync(dir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// entry guard — a gate that passes silently when misinvoked is worse than none
// ---------------------------------------------------------------------------

test('entry guard: the CLI still runs from a path with a space and through a symlink', () => {
  const src = fileURLToPath(new URL('./docs-lint.mjs', import.meta.url));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs lint '));
  const scripts = path.join(dir, 'my scripts');
  fs.mkdirSync(scripts);
  const copy = path.join(scripts, 'docs-lint.mjs');
  fs.copyFileSync(src, copy);
  fs.writeFileSync(path.join(dir, 'x.rs'), '//! F6 — the entry point.\n');

  /** @param {string} script */
  const cli = (script) =>
    spawnSync(process.execPath, [script, '--rules', 'plan-codes', 'x.rs'],
      { cwd: dir, encoding: 'utf8' });

  const spaced = cli(copy);
  assert.equal(spaced.status, 1, 'a space in the script path must not silently exit 0');
  assert.match(spaced.stdout, /x\.rs:1:5 \[E\] plan-codes/);

  const link = path.join(dir, 'linked.mjs');
  fs.symlinkSync(copy, link);
  const linked = cli(link);
  assert.equal(linked.status, 1, 'a symlinked script path must not silently exit 0');
  assert.match(linked.stdout, /x\.rs:1:5 \[E\] plan-codes/);

  const relative = spawnSync(process.execPath, ['./my scripts/docs-lint.mjs', '--rules', 'plan-codes', 'x.rs'],
    { cwd: dir, encoding: 'utf8' });
  assert.equal(relative.status, 1, 'a relative argv[1] must not silently exit 0');

  fs.rmSync(dir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// plan codes in identifiers: Rust types out, upper-case codes in
// ---------------------------------------------------------------------------

test('plan-codes: f32/f64 are Rust types and hex literals are not identifiers', () => {
  const src = [
    'fn scale(x: f32) -> f64 { x as f64 }',
    'const MASK: u8 = 0xF6;',
    'let masked = value & 0xF6_u8;',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/main.rs', src), 'plan-codes'), []);
});

test('plan-codes: upper-case and camelCase identifiers carry codes too', () => {
  const src = [
    'const F6_MODE: u8 = 1;',
    'const T7_PROBE: u8 = 2;',
    'fn parseF6Config() {}',
    'fn f6_missing_config() {}',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/main.rs', src), 'plan-codes'), ['F6', 'T7', 'F6', 'f6']);
});

// ---------------------------------------------------------------------------
// Rust doc comments are Markdown: spans, fences and `::` paths are code
// ---------------------------------------------------------------------------

test('rust doc comments: backtick spans and ``` fences are literals, not prose', () => {
  const src = [
    '//! The `F6` flag and `i64::MAX`, `PLAN.md`, `soak`, `dexpi4`, `NOT`.',
    '//! ```',
    '//! THE EXTENSION DECIDES; run the soak on dexpi4.',
    '//! ```',
    '//! The F6 flag is NOT optional during the soak.',
  ].join('\n');
  const f = lint('src/exhibit.rs', src);
  assert.deepEqual(f.map((x) => [x.line, x.rule, x.token]), [
    [5, 'plan-codes', 'F6'],
    [5, 'caps', 'NOT'],
    [5, 'coinages', 'soak'],
  ]);
});

test('rust doc comments: `::`-qualified paths are code even inside a sentence', () => {
  const src = '/// Returns i64::MAX on overflow, and libc::EINTR is retried.\n';
  assert.deepEqual(tokens(lint('src/main.rs', src), 'caps'), []);
});

// ---------------------------------------------------------------------------
// man pages, measurement record, quoted speech
// ---------------------------------------------------------------------------

test('caps: roff uppercases custom section headings, but man prose is still linted', () => {
  const src = [
    '.TH DEXD 1 "2026-08-17" "dexd 0.1.0" "dex"',
    '.SH WHY BOTH A KERNEL FORCE AND AN EXHIBIT CONFIG',
    'The kernel force is REQUIRED here.',
  ].join('\n');
  assert.deepEqual(tokens(lint('deploy/man/dexd.1', src), 'caps'), ['REQUIRED']);
});

test('caps: hyphenated codes like RP-3 and DS-1 are not caps emphasis', () => {
  assert.deepEqual(tokens(lint('docs/x.md', 'Cards RP-3 and DS-1 are the sources.\n'), 'caps'), []);
});

test('caps and dates: the measurement record keeps its verdict vocabulary and its dates', () => {
  const record = '| e04 | MET | PASS | FAIL | VOID | 2026-08-17 |\n\nThe NOT is emphasis.\n';
  const f = lint('docs/design/measurements.md', record);
  assert.deepEqual(tokens(f, 'caps'), ['NOT']);
  assert.deepEqual(of(f, 'dates'), []);
  const elsewhere = tokens(lint('docs/x.md', '| e04 | MET | PASS |\n'), 'caps');
  assert.ok(elsewhere.includes('MET') && elsewhere.includes('PASS'));
});

test('intensifiers: "honest" inside quoted speech is the speaker\'s word', () => {
  const quoted = '/// The prompt says "be honest with me" and nothing else.\n';
  assert.deepEqual(tokens(lint('src/main.rs', quoted), 'intensifiers'), []);
  const prose = '/// The honest count is 228 shared objects.\n';
  assert.deepEqual(tokens(lint('src/main.rs', prose), 'intensifiers'), ['honest']);
});

// ---------------------------------------------------------------------------
// coinage narrowing, bold around a literal, typographic marks
// ---------------------------------------------------------------------------

test('coinages: only "the pass bar" is the coinage, and it is reported once', () => {
  assert.deepEqual(tokens(lint('docs/x.md', 'The bar layer sits above the progress bar.\n'), 'coinages'), []);
  const f = lint('docs/x.md', 'Every card must clear the pass bar.\n');
  assert.deepEqual(tokens(f, 'coinages'), ['the pass bar']);
  assert.deepEqual(tokens(f, 'codenames'), [], 'no double report from codenames');
});

test('coinages: "hold" as an ordinary verb is not the coinage', () => {
  const src = '/// The decoder holds the last frame; hold the buffer until the flush.\n';
  assert.deepEqual(tokens(lint('src/main.rs', src), 'coinages'), []);
});

test('codenames: "thermal" is only a bench session next to tmux or session', () => {
  assert.deepEqual(tokens(lint('docs/x.md', 'Thermal throttling starts at 80 C.\n'), 'codenames'), []);
  assert.deepEqual(tokens(lint('docs/x.md', 'The thermal session owns the display.\n'), 'codenames'), ['thermal']);
});

test('codenames: "this run" means this process execution, not a bench session', () => {
  const src = '/// The ping counter is reset for this run; today it was not.\n';
  assert.deepEqual(tokens(lint('src/heartbeat.rs', src), 'codenames'), ['today']);
});

test('private-refs: PLAN inside a URL is a link target, not a citation', () => {
  const src = 'See [the upstream note](https://example.invalid/PLAN.md) for the wiring.\n';
  assert.deepEqual(tokens(lint('docs/x.md', src), 'private-refs'), []);
});

test('bold: bold wrapping only a code span is a literal, not emphasis', () => {
  const src = 'Use **`--force`** to override. ' + 'word '.repeat(100) + '\n';
  assert.deepEqual(of(lint('docs/x.md', src), 'bold'), []);
});

test('structure: (c), (r), (tm) and arrows are typography, not status emoji', () => {
  const src = '## Licence © 2026 ® ™\n\n- ↔ two-way sync\n- ✅ done\n';
  const f = of(lint('docs/x.md', src), 'structure');
  assert.deepEqual(f.map((x) => [x.line, x.token]), [[4, '✅']]);
});

// ---------------------------------------------------------------------------
// The gate must not fail on its own configuration, and must not truncate its
// own report. Both are the difference between a check CI can run and one it
// cannot.
// ---------------------------------------------------------------------------

test('cli: a report larger than the pipe buffer survives being piped', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs-lint-pipe-'));
  fs.mkdirSync(path.join(dir, 'docs'));
  const line = 'The F6 soak on dexpi4 is NOT the wrap.\n';
  fs.writeFileSync(path.join(dir, 'docs', 'big.md'), line.repeat(2000));

  // spawnSync gives the child a pipe for stdout, exactly like `| jq` or a CI
  // log capture. process.exit() would cut this off at 64 KB.
  const r = spawnSync(process.execPath,
    [fileURLToPath(new URL('./docs-lint.mjs', import.meta.url)), '--format', 'json', 'docs'],
    { cwd: dir, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });

  assert.equal(r.status, 1, 'errors still exit 1');
  assert.ok(r.stdout.length > 65536, `report should exceed the 64 KB pipe buffer, got ${r.stdout.length}`);
  const json = JSON.parse(r.stdout);          // throws if the tail was truncated
  assert.ok(json.length > 2000);
  assert.equal(json[json.length - 1].path, 'docs/big.md');

  fs.rmSync(dir, { recursive: true, force: true });
});

test('cli: the default run lints neither the glossary it reads nor the rulebook', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs-lint-self-'));
  fs.mkdirSync(path.join(dir, 'docs'));
  // The glossary retires the coinages in the form the parser reads them back.
  fs.writeFileSync(path.join(dir, 'docs', 'glossary.md'),
    '### loop point\nAlso written: seam, the wrap\n\nWhere playback returns to the first frame.\n');
  // The rulebook quotes every bad example verbatim, by design.
  fs.writeFileSync(path.join(dir, 'AGENTS.md'),
    '# Writing rules\n\n1. No plan codes: bad: F6 — the exhibit config. Run the soak on dexpi4.\n');
  fs.writeFileSync(path.join(dir, 'docs', 'guide.md'), '# Guide\n\nThe player restarts on failure.\n');

  /** @type {string[]} */
  let out = [];
  const io = { log: (s) => out.push(s), err: () => {}, cwd: dir };
  assert.equal(run([], io), 0, 'the documented CI invocation must pass on a clean tree');
  const text = out.join('\n');
  assert.doesNotMatch(text, /glossary\.md/);
  assert.doesNotMatch(text, /AGENTS\.md/);

  // Named explicitly, the rulebook is still lintable; the glossary never is.
  out = [];
  assert.equal(run(['AGENTS.md'], io), 1, 'AGENTS.md is skipped on a walk, not exempt when named');
  out = [];
  assert.equal(run(['docs/glossary.md'], io), 2, 'the glossary is this run\'s config: nothing to lint');
});

// ---------------------------------------------------------------------------
// Cargo.toml description fields: the .deb `Description:`, the most public text
// ---------------------------------------------------------------------------

test('toml: description and extended-description are prose, other values are not', () => {
  const src = [
    'description = "Gapless looper that removes the seam at the loop point"',
    'license-file = "LICENSE"',
    'version = "0.1.0-2026-08-17.abc"',
    'extended-description = """',
    'Feeds libmpv an endless byte stream, which is what removes the seam',
    'at the loop point."""',
    'section = "video"',
  ].join('\n');
  const f = lint('Cargo.toml', src);
  assert.deepEqual(of(f, 'coinages').map((x) => [x.line, x.token]), [[1, 'seam'], [5, 'seam']]);
  assert.deepEqual(of(f, 'dates'), [], 'a version string is not comment prose');
  assert.deepEqual(of(f, 'caps'), [], '"LICENSE" is a value, not prose');
});

test('toml: a single-line extended-description and a "#" comment both still work', () => {
  const src = [
    'extended-description = """Runs the 24 h soak."""',
    '# The DELIBERATE choice of $auto.',
  ].join('\n');
  const f = lint('Cargo.toml', src);
  assert.deepEqual(tokens(f, 'coinages'), ['soak']);
  assert.deepEqual(tokens(f, 'intensifiers'), ['DELIBERATE']);
});

// ---------------------------------------------------------------------------
// Fences: shell comments inside them are not headings, and a fence indented
// into a list item is still a fence
// ---------------------------------------------------------------------------

test('structure: a "#" comment inside a bash fence is not a Markdown heading', () => {
  const src = [
    '# Setup',
    '',
    '```bash',
    '# Verified on the Pi',
    '# ✅ works',
    '```',
    '',
    '- ✅ still a real status marker',
  ].join('\n');
  const f = of(lint('docs/x.md', src), 'structure');
  assert.deepEqual(f.map((x) => [x.line, x.severity]), [[8, 'W']]);
});

test('markdown: a fence indented into an ordered or nested list item is still code', () => {
  const src = [
    '1. Build it:',
    '',
    '    ```sh',
    '    # run the F6 soak on dexpi4 — THE EXTENSION DECIDES',
    '    ```',
    '',
    '   - and deeper:',
    '',
    '       ```',
    '       see PLAN.md for the wrap',
    '       ```',
    '',
    'But F6 out here is a hit.',
  ].join('\n');
  const f = lint('docs/x.md', src);
  assert.deepEqual(f.map((x) => [x.line, x.rule, x.token]), [[13, 'plan-codes', 'F6']]);
});

// ---------------------------------------------------------------------------
// Precision fixes: --bench-* CLI modes, metric fasteners
// ---------------------------------------------------------------------------

test('codenames: "the bench flag" is the --bench-only CLI mode, "the bench" is the room', () => {
  const cli = [
    '/// The bench flag skips the sidecar; the bench branch is the escape hatch.',
    '/// Pass --bench-only: the bench mode never writes, and the bench override is ignored.',
    '/// The bench-escape hatch exists for the same reason.',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/main.rs', cli), 'codenames'), []);

  const room = '/// Measured on the bench, and again on the bench Pi.\n';
  assert.deepEqual(tokens(lint('src/main.rs', room), 'codenames').map((s) => s.toLowerCase()),
    ['the bench', 'the bench pi']);
});

test('plan-codes: M2.5 standoffs and M3 screws are mounting hardware, not plan items', () => {
  const src = 'Fit the M2.5 standoffs and four M3 screws, then M4 bolts.\n';
  assert.deepEqual(tokens(lint('docs/x.md', src), 'plan-codes'), []);
  assert.deepEqual(tokens(lint('docs/x.md', 'M3 is the milestone.\n'), 'plan-codes'), ['M3']);
});

// ---------------------------------------------------------------------------
// Allowlist gate: a bare rule id would silence the rule everywhere
// ---------------------------------------------------------------------------

test('allowlist: a rule id with no token and no path= is refused', () => {
  assert.throws(() => parseAllowlist('caps # blanket\n'), ConfigError);
  assert.throws(() => parseAllowlist('first-person # we quote a lot\n'), ConfigError);
  // scoped-but-tokenless is a deliberate, readable exemption and stays legal
  const scoped = parseAllowlist('caps path=docs/design/measurements.md # verdict vocabulary\n');
  assert.equal(scoped.length, 1);
  assert.equal(scoped[0].tokenLiteral, null);
});

test('allowlist: the CLI exits 2 on a bare rule id', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs-lint-allow-'));
  fs.mkdirSync(path.join(dir, 'docs'));
  fs.writeFileSync(path.join(dir, 'docs', 'x.md'), 'The mode is NOT optional on dexpi4.\n');
  fs.writeFileSync(path.join(dir, 'docs', 'lint-allow.txt'), 'caps # blanket\n');
  const io = { log: () => {}, err: () => {}, cwd: dir };
  assert.equal(run(['docs'], io), 2);
  fs.rmSync(dir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Glossary headings that gloss the term inline
// ---------------------------------------------------------------------------

test('glossary: a heading may gloss its term with (), an em dash or a colon', () => {
  const g = [
    '### HEVC (High Efficiency Video Coding)',
    '### EDID — Extended Display Identification Data',
    '### KMS - Kernel Mode Setting',
    '### PATH: the executable search path',
    '### MIT-0',
    '### loop point',
  ].join('\n');
  const { acronyms, terms } = parseGlossary(g);
  for (const a of ['HEVC', 'EDID', 'KMS', 'PATH', 'MIT-0']) {
    assert.ok(acronyms.has(a), `${a} should be registered`);
  }
  assert.ok(!acronyms.has('HEVC (High'));
  assert.ok(terms.has('loop point'));

  const src = '//! The HEVC stream carries the EDID through KMS.\n';
  assert.deepEqual(tokens(lintSource('src/a.rs', src, makeCtx({ acronyms })), 'caps'), []);
});

// ---------------------------------------------------------------------------
// Man pages: font macros carry sentences
// ---------------------------------------------------------------------------

test('man: .B / .I / .BR / .IP arguments are prose, other macros are directives', () => {
  const src = [
    '.TH DEXD 1 "2026-08-17" "dexd 0.1.0" "dex"',
    '.SH EXHIBIT CONFIG SCHEMA',
    '.B the file extension decides which:',
    '.IP the soak runs for 24 h',
    '.br',
    '.RS 4',
  ].join('\n');
  const f = lint('deploy/man/dexd.1', src);
  assert.deepEqual(of(f, 'coinages').map((x) => [x.line, x.token]), [[4, 'soak']]);
  assert.equal(f.filter((x) => x.line >= 5).length, 0, '.br and .RS carry no prose');
});

// ---------------------------------------------------------------------------
// Gaps named by the verification: string literals, the "exactly" warning path,
// tables, unit Description=, JSON values, and the false-positive classes that
// had no fixture
// ---------------------------------------------------------------------------

test('rust: string literals are shipped text and reach every rule (§4.10)', () => {
  const src = [
    'fn help() {',
    '    println!("see PLAN.md F6 for the soak");',
    '    let banner = r#"THE EXTENSION DECIDES the parser"#;',
    '    let quote: char = \'"\';',
    '    let raw = br"we measured 0.753x realtime";',
    '}',
    'struct Holder<\'a> { name: &\'a str }',
    '/* nested /* comment */ mentioning dexpi4 */',
  ].join('\n');
  const f = lint('src/main.rs', src);
  const at = (rule) => of(f, rule).map((x) => [x.line, x.token]);
  assert.deepEqual(at('private-refs'), [[2, 'PLAN.md']]);
  assert.ok(at('plan-codes').some(([l, t]) => l === 2 && t === 'F6'));
  assert.ok(at('coinages').some(([l, t]) => l === 2 && t === 'soak'));
  assert.deepEqual(at('caps'), [[3, 'THE'], [3, 'EXTENSION'], [3, 'DECIDES']]);
  assert.deepEqual(at('process-talk'), [[5, 'we measured']]);
  assert.deepEqual(at('codenames'), [[8, 'dexpi4']], 'a nested block comment closes correctly');
});

test('intensifiers: "exactly" warns above 1 per 1000 words and is silent below it', () => {
  const dense = '/// The offset is exactly right.\n';
  const w = of(lint('src/chunk.rs', dense), 'intensifiers');
  assert.deepEqual(w.map((x) => [x.severity, x.token]), [['W', 'exactly']]);
  assert.match(w[0].message, /not in front of a number/);

  const sparse = '/// ' + 'word '.repeat(1500) + 'exactly right.\n';
  assert.deepEqual(of(lint('src/chunk.rs', sparse), 'intensifiers'), []);
});

test('intensifiers: "deliberate" as a verb is not the house intensifier', () => {
  const src = '/// The panel deliberates, and the deliberation is minuted.\n';
  assert.deepEqual(tokens(lint('docs/x.md', src), 'intensifiers'), []);
  assert.deepEqual(tokens(lint('docs/x.md', '/// The gap is deliberate.\n'), 'intensifiers'),
    ['deliberate']);
});

test('bold: bold inside a Markdown table is a column label, not emphasis', () => {
  const src = [
    '| Field | Meaning |',
    '| --- | --- |',
    '| **not** applied | the override was ignored |',
    '| **stops** | the unit stops |',
    '',
    'word '.repeat(20),
  ].join('\n');
  assert.deepEqual(of(lint('docs/x.md', src), 'bold'), []);
});

test('unit files: Description= is prose and is linted', () => {
  const src = [
    '[Unit]',
    'Description=We measured the gapless loop at the wrap',
    'After=network.target',
  ].join('\n');
  const f = lint('deploy/dexd.service', src);
  assert.deepEqual(tokens(f, 'first-person'), ['We']);
  assert.deepEqual(tokens(f, 'coinages'), ['at the wrap']);
  assert.deepEqual(tokens(f, 'caps'), [], 'After=network.target is a directive');
});

test('json config: string values are shipped text, keys and numbers are not', () => {
  const src = [
    '{',
    '  "note": "the value matches what ExecStart used to hardcode",',
    '  "soak_seconds": 86400,',
    '  "asset": "/var/lib/dex/loop.mp4"',
    '}',
  ].join('\n');
  const f = lint('deploy/exhibit.json.default', src);
  assert.deepEqual(tokens(f, 'process-talk'), ['used to hardcode']);
  assert.deepEqual(tokens(f, 'coinages'), [], 'the "soak_seconds" key is not prose');
});

test('caps: WATCHDOG=1 and NOTIFY_SOCKET are the interface, not emphasis', () => {
  const src = '/// Send WATCHDOG=1 to NOTIFY_SOCKET before READY=1, over AF_UNIX.\n';
  assert.deepEqual(tokens(lint('src/watchdog.rs', src), 'caps'), []);
});

test('dates: a copyright year line and a date inside a code span are not reports', () => {
  const licence = '# Copyright (c) 2026 Max Albrecht; reissued 2026-08-17.\n';
  assert.deepEqual(of(lint('deploy/maintainer-scripts/postinst', licence), 'dates'), []);
  const span = 'Run `git log --since=2026-08-01` to list them.\n';
  assert.deepEqual(of(lint('docs/x.md', span), 'dates'), []);
});

// ---------------------------------------------------------------------------
// Corpus calibration findings. Each one is a class of miss or false positive
// the gate shipped with; the fixture below is what stops it coming back.
// ---------------------------------------------------------------------------

test('cli: an input path that does not exist exits 2 instead of shrinking the scope', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docs-lint-missing-'));
  fs.mkdirSync(path.join(dir, 'docs'));
  fs.writeFileSync(path.join(dir, 'docs', 'guide.md'), 'The player restarts on failure.\n');

  /** @type {string[]} */
  const errs = [];
  const io = { log: () => {}, err: (s) => errs.push(s), cwd: dir };

  assert.equal(run(['docs', 'packages/dexd'], io), 2, 'a typo must not pass as a clean tree');
  assert.match(errs.join('\n'), /input path does not exist: packages\/dexd/);

  errs.length = 0;
  assert.equal(run(['docs'], io), 0, 'a path that is there still lints');

  // The built-in defaults are a superset of any one checkout, so they stay lenient.
  errs.length = 0;
  assert.equal(run([], io), 0, 'the default paths are not an explicit ask');

  fs.rmSync(dir, { recursive: true, force: true });
});

test('scan unit: a comment block is one string, so a wrapped pattern is seen whole', () => {
  const modDoc = [
    '//! journal-logged at startup so the note that used to',
    '//! live in a config comment travels with the config.',
  ].join('\n');
  assert.deepEqual(of(lint('src/exhibit.rs', modDoc), 'process-talk').map((x) => [x.line, x.col, x.token]),
    [[1, 48, 'used to live']]);

  const docComment = [
    '/// Each call site supplies its own article -- an earlier',
    '/// revision baked the article into the names.',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/exhibit.rs', docComment), 'process-talk'), ['an earlier revision']);

  const plain = [
    '    // by however many drops actually occurred (MINOR, three',
    '    // adversarial reviews). Recording the unavailability here',
  ].join('\n');
  const t = tokens(lint('src/main.rs', plain), 'process-talk');
  assert.ok(t.includes('three adversarial reviews'), `wrapped review count, got ${t}`);

  // A match starting on the second line still reports that line and column.
  const second = [
    '//! The unit file carries the',
    '//! deliberate default.',
  ].join('\n');
  assert.deepEqual(of(lint('src/a.rs', second), 'intensifiers').map((x) => [x.line, x.col, x.token]),
    [[2, 5, 'deliberate']]);
});

test('scan unit: the block ends where the writer ended it', () => {
  // Two remarks about two statements are two blocks, never one sentence.
  const trailing = [
    'let a = compute(); // the timeout used to',
    'let b = compute(); // live in the unit file.',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/a.rs', trailing), 'process-talk'), []);

  // A heading is its own unit: it never runs on into the paragraph below it.
  const heading = ['## The timeout used to', 'live in the unit file.'].join('\n');
  assert.deepEqual(tokens(lint('docs/x.md', heading), 'process-talk'), []);

  // Two lines of one paragraph are one sentence.
  const paragraph = ['The timeout used to', 'live in the unit file.'].join('\n');
  assert.deepEqual(of(lint('docs/x.md', paragraph), 'process-talk').map((x) => [x.line, x.col, x.token]),
    [[1, 13, 'used to live']]);
});

test('codenames: "the bench escape hatch" is the CLI mode even when it wraps', () => {
  const wrapped = [
    '/// Where a bound fps value came from: the sidecar, or the bench',
    '/// escape hatch (--bench-no-sidecar --fps <F>).',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/sidecar.rs', wrapped), 'codenames'), []);

  const room = ['/// Measured on the bench', '/// with the card attached.'].join('\n');
  assert.deepEqual(tokens(lint('src/sidecar.rs', room), 'codenames'), ['the bench']);
});

test('dates: "(later)" is a time-of-day split in a heading, a revision in a table', () => {
  const table = [
    '| Generation | Boards |',
    '| --- | --- |',
    '| G3 | Pi 2 B (later); Pi 3 B (later); CM3+ |',
  ].join('\n');
  assert.deepEqual(of(lint('docs/x.md', table), 'dates'), []);
  const prose = 'The B revision (later) carries the same silicon.\n';
  assert.deepEqual(of(lint('docs/x.md', prose), 'dates'), []);
  assert.deepEqual(tokens(lint('docs/x.md', '### The refusal path (later)\n'), 'dates'), ['(later)']);
});

test('codenames: naming the Cam Link 4K in full is the introduction, not the referent', () => {
  const full = 'An Elgato Cam Link 4K is the sink, and the Cam Link 4K forces the mode.\n';
  assert.deepEqual(tokens(lint('docs/x.md', full), 'codenames'), []);
  // The introduction and the short name in one sentence: still introduced.
  const sameLine = 'The Cam Link 4K capture card is the sink, so the Cam Link forces the mode.\n';
  assert.deepEqual(tokens(lint('docs/x.md', sameLine), 'codenames'), []);
  // With no introduction anywhere in the file it is still the bare referent.
  assert.deepEqual(tokens(lint('docs/y.md', 'So the Cam Link forces the mode.\n'), 'codenames'),
    ['the Cam Link']);
});

test('first-person: an all-caps run is not the pronoun, but "I" still is', () => {
  const shout = '/// EXTRACT US FROM THE LIST; MY OWN COPY AND OUR NOTES ARE HERE.\n';
  assert.deepEqual(tokens(lint('src/a.rs', shout), 'first-person'), []);
  const mixed = '/// I read it, and we kept our copy.\n';
  assert.deepEqual(tokens(lint('src/a.rs', mixed), 'first-person'), ['I', 'we', 'our']);
});

test('caps: a format placeholder and an environment reference name a constant', () => {
  const src = [
    'fn usage() -> ExitCode { eprint!("{USAGE}"); ExitCode::FAILURE }',
    'let m = format!("gave up after {ATTEMPTS} sends");',
    '// The cache is $HOME/.cache, and ${HOME}/.config is read too.',
    '// Bounded by ATTEMPTS, never by wall time.',
  ].join('\n');
  assert.deepEqual(tokens(lint('src/watchdog.rs', src), 'caps'), ['ATTEMPTS'],
    'only the bare constant reference in prose is emphasis');
});

test('code spans are literals in every comment kind and in string literals', () => {
  const rs = [
    '// The `soak` flag is documented; the soak is the run.',
    '/* Named `PLAN.md` in the fixture, but PLAN.md is the citation. */',
    'let msg = "pass `F6` on the command line; F6 is the code";',
  ].join('\n');
  const f = lint('src/main.rs', rs);
  assert.deepEqual(of(f, 'coinages').map((x) => [x.line, x.token]), [[1, 'soak']]);
  assert.deepEqual(of(f, 'private-refs').map((x) => [x.line, x.token]), [[2, 'PLAN.md']]);
  assert.deepEqual(of(f, 'plan-codes').map((x) => [x.line, x.token]), [[3, 'F6']]);

  const yml = '# The `soak` job is scheduled; the soak runs nightly.\n';
  assert.deepEqual(of(lint('.github/workflows/dexd.yml', yml), 'coinages').map((x) => x.token), ['soak']);
});

test('process-talk: the tail of an ISO date is not a review count', () => {
  const src = '/// Corrected at the 2026-08-17 review, after three independent reviews.\n';
  assert.deepEqual(tokens(lint('src/a.rs', src), 'process-talk'), ['three independent reviews']);
});
