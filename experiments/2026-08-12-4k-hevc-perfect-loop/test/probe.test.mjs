// @ts-check
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

/**
 * @param {string[]} extra
 * @returns {string[]}
 */
const argv = (extra) =>
  execFileSync('bash', ['scripts/probe.sh', '--asset', '/tmp/x.mp4', '--dry-run', ...extra])
    .toString().trim().split('\n');

test('loop-file config uses --loop-file=inf and never touches EOF handling flags', () => {
  const a = argv(['--config', 'loop-file']);
  assert.equal(a[0], 'mpv');
  assert.ok(a.includes('--loop-file=inf'));
  assert.ok(a.includes('/tmp/x.mp4'));
  assert.equal(a.some((t) => t.startsWith('--ab-loop')), false);
});

test('ab-loop config seeks before EOF using an explicit b point', () => {
  const a = argv(['--config', 'ab-loop', '--duration', '1.0']);
  assert.ok(a.includes('--ab-loop-a=0'));
  assert.ok(a.some((t) => /^--ab-loop-b=0\.9\d*$/.test(t)), `got ${a.join(' ')}`);
});

test('keep-open config opens an IPC socket for scripted seeking', () => {
  const a = argv(['--config', 'keep-open']);
  assert.ok(a.includes('--keep-open=yes'));
  assert.ok(a.some((t) => t.startsWith('--input-ipc-server=')));
});

test('every config is a kiosk: fullscreen, no OSC, no default bindings', () => {
  for (const cfg of ['loop-file', 'ab-loop', 'keep-open']) {
    const a = argv(['--config', cfg, '--duration', '1.0']);
    assert.ok(a.includes('--fullscreen'), `${cfg} missing --fullscreen`);
    assert.ok(a.includes('--no-osc'), `${cfg} missing --no-osc`);
    assert.ok(a.includes('--no-input-default-bindings'), `${cfg} missing --no-input-default-bindings`);
  }
});

test('vo and hwdec are overridable so the bench can enumerate combinations', () => {
  const a = argv(['--config', 'loop-file', '--vo', 'gpu', '--hwdec', 'v4l2request']);
  assert.ok(a.includes('--vo=gpu'));
  assert.ok(a.includes('--hwdec=v4l2request'));
});

test('an unknown config is rejected rather than silently defaulting', () => {
  assert.throws(() => argv(['--config', 'nonsense']), /unknown config/i);
});

test('ab-loop without --duration is rejected', () => {
  assert.throws(() => argv(['--config', 'ab-loop']), /--duration is required/i);
});

test('--stats adds a dump-stats flag', () => {
  const a = argv(['--config', 'loop-file', '--stats', '/tmp/s.txt']);
  assert.ok(a.includes('--dump-stats=/tmp/s.txt'));
});
