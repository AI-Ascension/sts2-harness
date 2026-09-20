// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, symlink, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { sha256 } from './contract.mjs';
import { absolutePath, environmentNames, inputPath, payload, schedule, validateRunner, validateInput } from './runner-contract.mjs';
import { preflight, selectedEnvironment } from './runner-io.mjs';
import { sortedCatalog } from './runner-evidence.mjs';
import { fixture } from './runner-test-fixtures.mjs';

for (const [name, value] of [['empty', ''], ['relative', 'Program Files/x'], ['parent', '/x/../y'],
  ['newline', '/x\ny'], ['NUL', '/x\0y'], ['oversize', '/' + 'x'.repeat(4096)]]) {
  test(`runner rejects ${name} executable locator`, () => assert.throws(() => absolutePath(value)));
}
test('runner preserves spaces, flag-like text and literal metacharacters without shell splitting', () => {
  assert.equal(absolutePath('/opt/Program Files/--gate 0;literal'), '/opt/Program Files/--gate 0;literal');
  assert.equal(inputPath('approved inputs/first.json'), 'approved inputs/first.json');
});
for (const path of ['../input.json', '/absolute.json', 'a//b', './x', 'a\\b', 'a/../b', 'a\nb']) {
  test(`rejects unsafe input path ${JSON.stringify(path)}`, () => assert.throws(() => inputPath(path)));
}
for (const name of ['NODE_OPTIONS', 'NODE_PATH', 'LD_PRELOAD', 'DYLD_LIBRARY_PATH', 'BASH_ENV',
  'PYTHONPATH', 'ENV', 'JEV_CONTEXT_LOG', 'lowercase', 'A=B']) {
  test(`environment rejects ${name}`, () => assert.throws(() => environmentNames([name])));
}
test('environment values are explicit, bounded and not inherited ambiently', () => {
  const m = { inherited_environment: ['TYPESAFE_API_KEY'] };
  assert.deepEqual(Object.keys(selectedEnvironment(m, { TYPESAFE_API_KEY: 'allowed', SECRET: 'hidden' })), ['TYPESAFE_API_KEY']);
  assert.throws(() => selectedEnvironment(m, {}));
  assert.throws(() => selectedEnvironment(m, { TYPESAFE_API_KEY: 'x\0y' }));
  assert.throws(() => environmentNames(['PATH', 'PATH']));
});

for (const [name, change] of [
  ['floating model', m => { m.model = 'jev-latest'; }],
  ['missing budget', m => { delete m.budgets.max_provider_attempts; }],
  ['unfunded pair', m => { m.budgets.max_provider_attempts = 1; }],
  ['excessive pairs', m => { m.budgets.max_pairs = 257; }],
  ['fractional gate', m => { m.confidence_gate_percent = 1.5; }],
  ['gate overflow', m => { m.confidence_gate_percent = 101; }],
  ['unbounded timeout', m => { m.budgets.total_timeout_ms = 3600001; }],
  ['unknown field', m => { m.secret = 'not allowed'; }],
  ['duplicate pair', m => { m.pairs.push(structuredClone(m.pairs[0])); }],
  ['cluster split leakage', m => { m.pairs.push({ ...m.pairs[0], pair_id: 'second',
    input_sha256: 'c'.repeat(64), repetition: 1, split: 'calibration' }); }],
]) {
  test(`manifest rejects ${name}`, { skip: process.platform === 'win32' }, async t => {
    const f = await fixture(t); change(f.manifest); assert.throws(() => validateRunner(f.manifest));
  });
}

test('execution IDs differ by arm and freeze deterministic balanced pair order', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t), hash = 'a'.repeat(64);
  const entries = schedule(f.manifest, hash);
  assert.deepEqual(entries, schedule(f.manifest, hash));
  assert.notEqual(entries[0].execution_id_digest, entries[1].execution_id_digest);
  assert.notEqual(entries[0].arm, entries[1].arm);
  const a = JSON.parse(payload(f.input, hash, 0)), b = JSON.parse(payload(f.input, hash, 1));
  assert.notEqual(a.model_execution_id, b.model_execution_id);
  delete a.model_execution_id; delete b.model_execution_id;
  assert.deepEqual(a, b); assert.equal(f.input.model_execution_id, 'approved-original');
});

test('stdout indices follow Rust UTF-8 order rather than JavaScript UTF-16 order', () => {
  assert.deepEqual(sortedCatalog(['\u{10000}', '\ue000', 'a']), ['a', '\ue000', '\u{10000}']);
});

test('request validation and post-ID-injection payload bounds refuse malformed data', () => {
  assert.throws(() => validateInput({ observation: {}, model_execution_id: 'x', legal_action_ids: ['x', 'x'] }));
  assert.throws(() => validateInput({ observation: {}, model_execution_id: 'x', legal_action_ids: ['a\nb'] }));
  assert.throws(() => payload({ raw: 'x'.repeat(131072) }, 'a'.repeat(64), 0));
});

for (const [name, mutate] of [
  ['input bytes change', async f => writeFile(f.inputPath, '{}\n')],
  ['world-readable input', async f => f.chmod(f.inputPath, 0o644)],
  ['world-readable manifest', async f => f.chmod(f.path, 0o644)],
  ['unsafe output parent', async f => { await mkdir(join(f.root, 'public'), { mode: 0o755 }); f.manifest.output_directory = join(f.root, 'public/run'); await f.save(); }],
  ['input symlink', async f => { await symlink(f.inputPath, join(f.root, 'linked.json')); f.manifest.pairs[0].input_path = 'linked.json'; await f.save(); }],
  ['bridge drift', async f => writeFile(f.bridge, 'changed')],
  ['group-writable executable', async f => f.chmod(f.transport, 0o770)],
  ['total input budget', async f => { f.manifest.budgets.max_total_input_bytes = 1; await f.save(); }],
]) {
  test(`preflight refuses ${name} without launching`, { skip: process.platform === 'win32' }, async t => {
    const f = await fixture(t); await mutate(f); await assert.rejects(preflight(f.path));
  });
}

test('semantic split leakage cannot be hidden by whitespace or execution IDs', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t);
  const bytes = Buffer.from(JSON.stringify({ ...f.input, model_execution_id: 'other' }, null, 2));
  await writeFile(join(f.root, 'other.json'), bytes, { mode: 0o600 });
  f.manifest.pairs.push({ ...f.manifest.pairs[0], pair_id: 'other', input_path: 'other.json',
    input_sha256: sha256(bytes), cluster_sha256: 'c'.repeat(64), split: 'calibration' });
  await f.save(); await assert.rejects(preflight(f.path));
});

test('preflight does not modify approved input bytes', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t), original = await readFile(f.inputPath);
  const p = await preflight(f.path); assert.equal(p.entries.length, 2);
  assert.deepEqual(await readFile(f.inputPath), original);
});
