// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { lstat, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { executeRun, planRun } from './paired-runner.mjs';
import { inspectRun } from './runner-journal.mjs';
import { fixture } from './runner-test-fixtures.mjs';

const unix = { skip: process.platform === 'win32' };
const json = async path => JSON.parse(await readFile(path, 'utf8'));
const execute = async f => executeRun(f.path, (await planRun(f.path)).manifest_sha256, { environment: f.environment });

test('planning validates pins and inputs but creates no output and launches no child', unix, async t => {
  const f = await fixture(t), plan = await planRun(f.path);
  assert.equal(plan.execution_performed, false); assert.equal(plan.scheduled_arms, 2);
  assert.equal(plan.maximum_reserved_provider_attempts, 2);
  assert.deepEqual(await readdir(f.proof), []);
  await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
});

test('an approval mismatch refuses before reserving or launching', unix, async t => {
  const f = await fixture(t);
  await assert.rejects(executeRun(f.path, '0'.repeat(64), { environment: f.environment }));
  assert.deepEqual(await readdir(f.proof), []);
  await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
});

test('missing declared environment refuses before output creation', unix, async t => {
  const f = await fixture(t), plan = await planRun(f.path);
  await assert.rejects(executeRun(f.path, plan.manifest_sha256, { environment: {} }));
  await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
});

test('independent paired execution preserves both inputs and integrates the existing capture reader', unix, async t => {
  const f = await fixture(t), original = await readFile(f.inputPath), result = await execute(f);
  assert.equal(result.counts.complete, 2); assert.equal(result.incomplete, false);
  assert.equal(result.reserved_provider_attempts, 2);
  assert.equal(result.observed_provider_attempts_known_sum, 2);
  assert.equal(result.by_arm.baseline.input_tokens_known_sum, 100);
  assert.equal(result.by_arm.tactical.input_tokens_known_sum, 100);
  assert.equal(result.by_arm.baseline.input_tokens_unknown_arms, 0);
  assert.equal(result.gameplay_improvement_established, false);
  const audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(audit.counts.disagree, 1); assert.equal(audit.independent_action_pairs, 1);
  assert.equal(audit.metrics.within_request_action_pairs, 1);
  assert.equal((await json(join(f.manifest.output_directory, 'audit-held_out.json'))).scheduled_pairs, 1);
  const proofs = await Promise.all((await readdir(f.proof)).sort().map(name => json(join(f.proof, name))));
  assert.equal(proofs.length, 2);
  assert.notEqual(proofs[0].input.model_execution_id, proofs[1].input.model_execution_id);
  for (const proof of proofs) {
    delete proof.input.model_execution_id;
    assert.ok(proof.args.includes(f.transport)); assert.equal(proof.args.includes('--record'), false);
    assert.equal(proof.secret_present, true);
    assert.equal(proof.environment_names.includes('UNDECLARED_SECRET'), false);
    assert.equal(proof.environment_names.includes('HOME'), false);
  }
  assert.deepEqual(proofs[0].input, proofs[1].input);
  assert.deepEqual(await readFile(f.inputPath), original);
  assert.equal((await inspectRun(f.manifest.output_directory)).counts.complete, 2);
});

test('create-only outputs prevent rerunning a spent experiment', unix, async t => {
  const f = await fixture(t), plan = await planRun(f.path); await execute(f);
  await assert.rejects(executeRun(f.path, plan.manifest_sha256, { environment: f.environment }));
  assert.equal((await readdir(f.proof)).length, 2);
});

test('runner artifacts have private modes and retain no raw decisions, state, paths or secrets', unix, async t => {
  const f = await fixture(t); await execute(f);
  async function walk(path) {
    const stat = await lstat(path);
    assert.equal(stat.mode & 0o7777, stat.isDirectory() ? 0o700 : 0o600);
    if (stat.isDirectory()) {
      for (const name of await readdir(path)) await walk(join(path, name));
    } else {
      const text = await readFile(path, 'utf8');
      for (const value of ['private-marker', 'synthetic-state', 'synthetic-secret', 'action-a',
        'action-b', 'approved-original', f.root]) assert.equal(text.includes(value), false, value);
    }
  }
  await walk(f.manifest.output_directory);
});

for (const [mode, terminal, pairStatus] of [
  ['capture_failed', 'bridge_failed', 'failed'],
  ['stderr_failure', 'bridge_failed', 'pending'],
  ['bad_identity', 'capture_invalid', 'unreported'],
  ['invalid_stdout', 'stdout_mismatch', 'unreported'],
  ['bad_stdout', 'stdout_mismatch', 'unreported'],
  ['extra_file', 'capture_invalid', 'unreported'],
  ['exit_after_complete', 'bridge_failed', 'unreported'],
  ['no_capture', 'capture_missing', 'unreported'],
  ['flood', 'output_bound', 'pending'],
]) {
  test(`${mode} is counted, never retried or admitted as an action pair`, unix, async t => {
    const f = await fixture(t, mode), result = await execute(f);
    assert.equal(result.counts[terminal], 2); assert.equal(result.incomplete, true);
    assert.equal((await readdir(f.proof)).length, 2);
    const audit = await json(join(f.manifest.output_directory, 'audit.json'));
    assert.equal(audit.counts[pairStatus], 1); assert.equal(audit.independent_action_pairs, 0);
  });
}

for (const [mode, status] of [['model_drift', 'model_drift'], ['input_drift', 'input_mismatch'],
  ['low_evidence', 'refusal']]) {
  test(`${mode} stays visible without contaminating independent agreement`, unix, async t => {
    const f = await fixture(t, mode), result = await execute(f);
    assert.equal(result.counts.complete, 2); assert.equal(result.incomplete, true);
    const audit = await json(join(f.manifest.output_directory, 'audit.json'));
    assert.equal(audit.counts[status], 1); assert.equal(audit.independent_action_pairs, 0);
  });
}

test('forced decisions consume reservations but have known zero provider attempts', unix, async t => {
  const f = await fixture(t); f.input.legal_action_ids = ['only']; await f.saveInput(); await f.save();
  const result = await execute(f);
  assert.equal(result.reserved_provider_attempts, 2); assert.equal(result.observed_provider_attempts_known_sum, 0);
  assert.equal((await json(join(f.manifest.output_directory, 'audit.json'))).counts.forced, 1);
});

test('artifact replacement between arms stops execution without silently re-pinning', unix, async t => {
  const f = await fixture(t, 'mutate_transport'), result = await execute(f);
  assert.equal(result.counts.complete, 1); assert.equal(result.counts.executable_drift, 1);
  assert.equal(result.reserved_provider_attempts, 1); assert.equal((await readdir(f.proof)).length, 1);
});

test('a global time budget bounds the first arm and leaves later arms explicitly unstarted', unix, async t => {
  const f = await fixture(t, 'timeout');
  f.manifest.budgets.total_timeout_ms = 1000; f.manifest.budgets.per_arm_timeout_ms = 3000;
  await f.save(); const result = await execute(f);
  const first = await json(join(f.manifest.output_directory, 'slot-0000.result.json'));
  const second = await json(join(f.manifest.output_directory, 'slot-0001.result.json'));
  assert.equal(result.counts.timeout, 1); assert.equal(result.counts.cancelled, 0);
  assert.equal(result.counts.not_started, 1);
  assert.equal(result.reserved_provider_attempts, 1);
  assert.equal(result.observed_provider_attempts_unknown_arms, 1);
  assert.equal(first.status, 'timeout'); assert.equal(first.process_started, true);
  assert.equal(first.reserved_provider_attempts, 1); assert.equal(first.observed_provider_attempts, null);
  assert.equal(second.status, 'not_started'); assert.equal(second.process_started, false);
  assert.equal(second.reserved_provider_attempts, 0); assert.equal(second.observed_provider_attempts, 0);
});

test('an interruption after the reservation cancels an arm instead of timing it out', unix, async t => {
  const f = await fixture(t, 'timeout');
  f.manifest.budgets.per_arm_timeout_ms = 120000; await f.save();
  const plan = await planRun(f.path), controller = new AbortController();
  const running = executeRun(f.path, plan.manifest_sha256,
    { signal: controller.signal, environment: f.environment });
  const pending = join(f.manifest.output_directory, 'slot-0000.pending.json');
  let reserved = false;
  for (let attempt = 0; attempt < 4000 && !reserved; attempt += 1) {
    reserved = await lstat(pending).then(() => true, () => false);
    if (!reserved) await new Promise(resolve => setTimeout(resolve, 5));
  }
  assert.equal(reserved, true); controller.abort();
  const result = await running;
  assert.equal(result.counts.cancelled, 1); assert.equal(result.counts.timeout, 0);
  assert.equal(result.counts.not_started, 1); assert.equal(result.reserved_provider_attempts, 1);
});

test('pre-cancelled runs perform no provider work and account for every arm', unix, async t => {
  const f = await fixture(t), plan = await planRun(f.path), controller = new AbortController();
  controller.abort();
  const result = await executeRun(f.path, plan.manifest_sha256, { signal: controller.signal, environment: f.environment });
  assert.equal(result.counts.not_started, 2); assert.equal(result.reserved_provider_attempts, 0);
  assert.equal((await readdir(f.proof)).length, 0);
});

test('read-only inspection keeps outstanding reservations unknown and never resumes them', unix, async t => {
  const f = await fixture(t); await execute(f);
  await rm(join(f.manifest.output_directory, 'slot-0001.result.json'));
  const before = await readdir(f.manifest.output_directory), proofs = await readdir(f.proof);
  const report = await inspectRun(f.manifest.output_directory);
  assert.equal(report.counts.interrupted_unknown, 1);
  assert.equal(report.observed_provider_attempts_unknown_arms, 1);
  assert.deepEqual(await readdir(f.manifest.output_directory), before);
  assert.deepEqual(await readdir(f.proof), proofs);
});

test('corrupted terminal journals are invalid, not silently reclassified as missing', unix, async t => {
  const f = await fixture(t); await execute(f);
  await writeFile(join(f.manifest.output_directory, 'slot-0000.result.json'), '{corrupted');
  await assert.rejects(inspectRun(f.manifest.output_directory));
});

test('CLI requires explicit run approval and emits only redacted errors', unix, async t => {
  const f = await fixture(t), cli = fileURLToPath(new URL('./runner-cli.mjs', import.meta.url));
  const invalid = spawnSync(process.execPath, [cli, 'run', f.path], { encoding: 'utf8' });
  assert.equal(invalid.status, 2); assert.equal(invalid.stdout, '');
  const plan = spawnSync(process.execPath, [cli, 'plan', f.path], { encoding: 'utf8' });
  assert.equal(plan.status, 0); assert.equal(JSON.parse(plan.stdout).execution_performed, false);
  const absent = spawnSync(process.execPath, [cli, 'plan', join(f.root, 'private-missing')], { encoding: 'utf8' });
  assert.equal(absent.status, 2); assert.equal(absent.stderr.includes(f.root), false);
  assert.equal(absent.stdout, ''); assert.equal((await readdir(f.proof)).length, 0);
});
