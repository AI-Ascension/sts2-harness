// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { planPilot } from './pilot.mjs';
import { validatePilot } from './pilot-profile.mjs';
import { json, pilotFixture, writeJson } from './pilot-test-fixtures.mjs';

const unix = { skip: process.platform === 'win32' };

test('pilot planning is read-only, bounded and does not need a provider credential', unix, async t => {
  const f = await pilotFixture(t), before = await readdir(f.root), bytes = await readFile(f.path);
  const report = await planPilot(f.path);
  assert.equal(report.scheduled_arms, 20); assert.equal(report.execution_performed, false);
  assert.equal(report.budgets.max_provider_attempts, 20);
  assert.equal(report.provider_availability_verified, false);
  assert.equal(report.cohort.declared_clusters, 10);
  assert.equal(report.inputs.distinct_inputs_excluding_execution_id, 10);
  assert.equal(report.splits.held_out.scheduled_pairs, 5);
  assert.deepEqual(await readdir(f.root), before); assert.deepEqual(await readFile(f.path), bytes);
  await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
  const text = JSON.stringify(report);
  for (const value of [f.root, 'PRIVATE_OBJECTIVE', 'private-action', 'private-pair', 'TYPESAFE_API_KEY']) {
    assert.equal(text.includes(value), false);
  }
});

for (const [name, change] of [
  ['fewer pairs', m => m.pairs.pop()],
  ['larger reservation ceiling', m => m.budgets.max_provider_attempts = 22],
  ['larger pair ceiling', m => m.budgets.max_pairs = 11],
  ['execution longer than thirty minutes', m => m.budgets.total_timeout_ms = 1800001],
  ['original-input budget above ten bounded inputs', m => m.budgets.max_total_input_bytes = 1310721],
  ['rolling model alias', m => m.model = 'jev-latest'],
  ['extra policy field', m => m.weights = [1]],
]) {
  test(`pilot rejects ${name}`, unix, async t => {
    const f = await pilotFixture(t); change(f.manifest); await f.save();
    assert.throws(() => validatePilot(f.manifest));
    await assert.rejects(planPilot(f.path));
    await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
  });
}

test('repetitions remain ten pairs but are not represented as ten independent inputs', unix, async t => {
  const f = await pilotFixture(t), first = f.manifest.pairs[0];
  f.manifest.pairs = Array.from({ length: 10 }, (_, repetition) => ({ ...first,
    pair_id: `pair-${repetition}`, repetition }));
  await f.save(); const report = await planPilot(f.path);
  assert.equal(report.cohort.scheduled_pairs, 10); assert.equal(report.cohort.declared_clusters, 1);
  assert.equal(report.cohort.distinct_input_file_hashes, 1);
  assert.equal(report.inputs.distinct_inputs_excluding_execution_id, 1);
  assert.equal(report.cohort.nonzero_repetition_pairs, 9);
});

test('semantic duplicates cannot cross splits under different execution IDs', unix, async t => {
  const f = await pilotFixture(t), first = await json(join(f.root, f.manifest.pairs[0].input_path));
  first.model_execution_id = 'different-but-not-independent';
  f.manifest.pairs[5].input_sha256 = await writeJson(join(f.root, f.manifest.pairs[5].input_path), first);
  await f.save(); await assert.rejects(planPilot(f.path), { code: 'runner_split_leakage' });
});

test('known non-ASCII identifier incompatibility is rejected without dropping a pair', unix, async t => {
  const f = await pilotFixture(t), pair = f.manifest.pairs[0], path = join(f.root, pair.input_path);
  const input = await json(path); input.legal_action_ids[0] = 'action-😀';
  pair.input_sha256 = await writeJson(path, input); await f.save();
  await assert.rejects(planPilot(f.path), { code: 'pilot_non_ascii_action_id' });
  assert.equal(f.manifest.pairs.length, 10);
});

test('forced and potentially oversized tactical catalogs remain visible in the plan', unix, async t => {
  const f = await pilotFixture(t, ['forced', 'fallback']), report = await planPilot(f.path);
  assert.deepEqual(report.inputs.raw_catalog_size_counts,
    { single_action: 1, two_to_twenty_four: 8, over_twenty_four: 1 });
});

test('input mutation and executable drift are refused by the existing preflight', unix, async t => {
  const f = await pilotFixture(t);
  await writeFile(join(f.root, f.manifest.pairs[0].input_path), '{}');
  await assert.rejects(planPilot(f.path), { code: 'runner_input_digest' });
  await writeFile(f.manifest.bridge.path, '#!/bin/false\n#changed\n');
  await assert.rejects(planPilot(f.path), { code: 'runner_executable_digest' });
});

test('CLI has no execution mode and errors contain no private locator', unix, async t => {
  const f = await pilotFixture(t), cli = fileURLToPath(new URL('./pilot-cli.mjs', import.meta.url));
  const plan = spawnSync(process.execPath, [cli, 'plan', f.path], { encoding: 'utf8' });
  assert.equal(plan.status, 0); assert.equal(JSON.parse(plan.stdout).execution_performed, false);
  for (const args of [['run', f.path], ['report', join(f.root, 'missing')], ['plan', f.path, 'extra']]) {
    const result = spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });
    assert.equal(result.status, 2); assert.equal(result.stdout, '');
    assert.equal(result.stderr.includes(f.root), false);
  }
});
