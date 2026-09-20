// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { runBridge } from './runner-process.mjs';
import { fixture } from './runner-test-fixtures.mjs';

const unix = { skip: process.platform === 'win32' };
function invoke(root, script, { timeoutMs = 1500, signal, bytes = Buffer.alloc(0), args = [] } = {}) {
  return runBridge(process.execPath, ['-e', script, ...args], bytes,
    { cwd: root, env: {}, timeoutMs, signal });
}

test('process boundary preserves one argument per path and metacharacters are literal', unix, async t => {
  const f = await fixture(t);
  const args = ['path with spaces', ';echo not-a-shell', '$HOME', '--gate 0'];
  const result = await invoke(f.root, 'process.stdout.write(JSON.stringify(process.argv.slice(1)))', { args });
  assert.equal(result.status, 'complete'); assert.equal(result.child_closed, true);
  assert.deepEqual(JSON.parse(result.stdout), args);
});

test('the process deadline includes unread stdin larger than a pipe', unix, async t => {
  const f = await fixture(t), start = performance.now();
  const result = await invoke(f.root, 'setInterval(() => {}, 1000)',
    { timeoutMs: 150, bytes: Buffer.alloc(131072) });
  assert.equal(result.status, 'timeout'); assert.equal(result.child_closed, true);
  assert.ok(performance.now() - start < 1800);
});

for (const [name, script] of [
  ['stdout', 'process.stdout.write("s".repeat(9000))'],
  ['stderr', 'process.stderr.write("s".repeat(70000))'],
]) {
  test(`${name} overflow is bounded and raw output is discarded`, unix, async t => {
    const f = await fixture(t), result = await invoke(f.root, script);
    assert.equal(result.status, 'output_bound'); assert.equal(result.stdout.length, 0);
    assert.equal(result.child_closed, true);
  });
}

test('failure output and stderr are never retained in a process result', unix, async t => {
  const f = await fixture(t);
  const result = await invoke(f.root,
    'process.stdout.write("synthetic-private-output"); process.stderr.write("synthetic-secret"); process.exit(2)');
  assert.equal(result.status, 'bridge_failed');
  assert.equal(result.stdout.length, 0); assert.equal(JSON.stringify(result).includes('synthetic-secret'), false);
});

test('nonexistent executable is an explicit spawn failure', unix, async t => {
  const f = await fixture(t), result = await runBridge(f.root + '/absent', [], Buffer.alloc(0),
    { cwd: f.root, env: {}, timeoutMs: 1000 });
  assert.equal(result.status, 'spawn_failed'); assert.equal(result.process_started, false);
});

test('pre-cancelled work does not launch a process', unix, async t => {
  const f = await fixture(t), controller = new AbortController(); controller.abort();
  const result = await invoke(f.root, 'process.exit(99)', { signal: controller.signal });
  assert.equal(result.status, 'cancelled'); assert.equal(result.process_started, false);
});

test('cancellation terminates a running process rather than retrying it', unix, async t => {
  const f = await fixture(t), controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 100); t.after(() => clearTimeout(timer));
  const result = await invoke(f.root, 'setInterval(() => {}, 1000)', { signal: controller.signal });
  assert.equal(result.status, 'cancelled'); assert.equal(result.child_closed, true);
});

test('a descendant retaining inherited pipes cannot bypass the total process deadline', unix, async t => {
  const f = await fixture(t), start = performance.now();
  const script = `require('node:child_process').spawn(process.execPath,
    ['-e', 'setTimeout(() => {}, 2500)'], {stdio:'inherit'}).unref();`;
  const result = await invoke(f.root, script, { timeoutMs: 300 });
  assert.equal(result.status, 'timeout');
  assert.equal(result.child_closed, true); // Same-group descendant pipes were closed after group termination.
  assert.ok(performance.now() - start < 1800);
});
