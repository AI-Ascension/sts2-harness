// SPDX-License-Identifier: MIT

// Real compiled Rust bridge + existing paired runner + a socket-free synthetic transport.
// Explicitly invoked with a built binary; a missing bridge is a failure, never a skipped test.
import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, copyFile, lstat, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { sha256 } from './contract.mjs';
import { executeRun, planRun } from './paired-runner.mjs';
import { inspectRun } from './runner-journal.mjs';
import { fixture } from './runner-test-fixtures.mjs';

const binary = process.env.STS2_JEV_TEST_BRIDGE;
const revision = process.env.STS2_JEV_TEST_SOURCE_REVISION;
assert.notEqual(process.platform, 'win32', 'compiled capture integration requires Unix');
assert.ok(binary && isAbsolute(binary), 'STS2_JEV_TEST_BRIDGE must name an absolute compiled binary');
assert.match(revision ?? '', /^[0-9a-f]{40}$/, 'declare the actual build checkout revision');
assert.equal(await realpath(binary), binary, 'compiled binary locator must be canonical');
assert.ok((await lstat(binary)).isFile(), 'compiled binary must be a regular file');
const binaryDigest = sha256(await readFile(binary));
const bounded = { timeout: 45000 };
const json = async path => JSON.parse(await readFile(path, 'utf8'));

async function compiledFixture(t, mode = 'success') {
  const f = await fixture(t, mode);
  // Use the exact built bytes, including in a path with spaces; do not substitute a bridge oracle.
  await copyFile(binary, f.bridge); await chmod(f.bridge, 0o700);
  const source = await readFile(new URL('./compiled-bridge-transport.mjs', import.meta.url));
  await writeFile(f.transport, Buffer.concat([Buffer.from(`#!${process.execPath}\n`), source]), { mode: 0o700 });
  f.manifest.bridge.sha256 = sha256(await readFile(f.bridge));
  assert.equal(f.manifest.bridge.sha256, binaryDigest);
  f.manifest.transport.sha256 = sha256(await readFile(f.transport));
  f.manifest.source_revision = revision;
  f.manifest.budgets.per_arm_timeout_ms = 5000;
  f.manifest.budgets.total_timeout_ms = 20000;
  Object.assign(f.input.observation.player, { max_hp: 80, energy: 3, gold: 0 });
  f.input.observation.state.turn_index = 2;
  await f.saveInput(); await f.save();
  return f;
}

async function execute(f) {
  return executeRun(f.path, (await planRun(f.path)).manifest_sha256, { environment: f.environment });
}

async function captures(f) {
  const plan = await json(join(f.manifest.output_directory, 'run.pending.json'));
  return Promise.all(plan.scheduled.map(async entry => ({ arm: entry.arm,
    record: await json(join(f.manifest.output_directory,
      `slot-${String(entry.slot).padStart(4, '0')}`, 'attempt-0000.result.json')),
  })));
}

async function proofs(f) {
  return Promise.all((await readdir(f.proof)).map(name => json(join(f.proof, name))));
}

async function privateArtifacts(f, path = f.manifest.output_directory) {
  const stat = await lstat(path);
  assert.equal(stat.mode & 0o7777, stat.isDirectory() ? 0o700 : 0o600);
  if (stat.isDirectory()) {
    for (const name of await readdir(path)) await privateArtifacts(f, join(path, name));
  } else {
    const text = await readFile(path, 'utf8');
    for (const value of ['private-marker', 'synthetic-state', 'synthetic-secret', 'action-a',
      'action-b', 'approved-original', 'SYNTHETIC_PRIVATE_TRANSPORT_ERROR', f.root]) {
      assert.equal(text.includes(value), false, 'raw synthetic content escaped into runner artifacts');
    }
  }
}

test('compiled bridge produces independent choices through the real paired capture/audit chain', bounded, async t => {
  const f = await compiledFixture(t), original = await readFile(f.inputPath);
  const plan = await planRun(f.path);
  assert.equal(plan.execution_performed, false);
  assert.deepEqual(await proofs(f), []);
  await assert.rejects(lstat(f.manifest.output_directory), { code: 'ENOENT' });
  const result = await executeRun(f.path, plan.manifest_sha256, { environment: f.environment });
  assert.equal(result.incomplete, false); assert.equal(result.counts.complete, 2);
  assert.equal(result.reserved_provider_attempts, 2);
  assert.equal(result.observed_provider_attempts_known_sum, 2);
  assert.equal(result.gameplay_improvement_established, false);
  const byArm = Object.fromEntries((await captures(f)).map(row => [row.arm, row.record]));
  assert.equal(byArm.baseline.profile, 'baseline');
  assert.equal(byArm.tactical.profile, 'jev-tactical-v1');
  assert.equal(byArm.baseline.bridge_digest, binaryDigest);
  assert.equal(byArm.tactical.bridge_digest, binaryDigest);
  assert.equal(byArm.baseline.input_digest, byArm.tactical.input_digest);
  assert.notEqual(byArm.baseline.model_execution_id_digest, byArm.tactical.model_execution_id_digest);
  assert.equal(byArm.baseline.decision.selected_index, 0);
  assert.equal(byArm.tactical.decision.selected_index, 1);
  assert.equal(byArm.tactical.tactical.applied, true);
  const proof = await proofs(f);
  assert.equal(proof.length, 2);
  assert.deepEqual(proof.map(row => row.question_count).sort((a, b) => a - b), [1, 15]);
  assert.equal(proof[0].state_sha256, proof[1].state_sha256);
  assert.equal(proof[0].action_question_sha256, proof[1].action_question_sha256);
  assert.notEqual(proof[0].request_sha256, proof[1].request_sha256);
  for (const row of proof) {
    assert.equal(row.synthetic_credential, true);
    assert.equal(row.environment_names.includes('UNDECLARED_SECRET'), false);
    assert.equal(row.environment_names.includes('HOME'), false);
  }
  const audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(audit.counts.disagree, 1); assert.equal(audit.independent_action_pairs, 1);
  assert.equal(audit.metrics.within_request_action_pairs, 1);
  for (const arm of ['baseline', 'tactical']) {
    assert.equal(result.by_arm[arm].input_tokens_known_sum, 100);
    assert.equal(result.by_arm[arm].input_tokens_unknown_arms, 0);
  }
  assert.deepEqual(await readFile(f.inputPath), original);
  assert.equal((await inspectRun(f.manifest.output_directory)).counts.complete, 2);
  await privateArtifacts(f);
  await assert.rejects(executeRun(f.path, plan.manifest_sha256, { environment: f.environment }));
  assert.equal((await proofs(f)).length, 2, 'spent run must not launch another transport');
});

for (const mode of ['malformed', 'transport_failure']) {
  test(`compiled bridge accounts for ${mode} without retry or action admission`, bounded, async t => {
    const f = await compiledFixture(t, mode), result = await execute(f);
    assert.equal(result.incomplete, true); assert.equal(result.counts.bridge_failed, 2);
    assert.equal(result.observed_provider_attempts_known_sum, 2);
    assert.equal((await proofs(f)).length, 2);
    const audit = await json(join(f.manifest.output_directory, 'audit.json'));
    assert.equal(audit.counts.failed, 1); assert.equal(audit.independent_action_pairs, 0);
    await privateArtifacts(f);
  });
}

test('missing tactical answer preserves the independently successful baseline', bounded, async t => {
  const f = await compiledFixture(t, 'missing_answer'), result = await execute(f);
  assert.equal(result.counts.complete, 1); assert.equal(result.counts.bridge_failed, 1);
  assert.equal(result.observed_provider_attempts_known_sum, 2);
  assert.equal((await proofs(f)).length, 2);
  const audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(audit.counts.failed, 1); assert.equal(audit.independent_action_pairs, 0);
});

for (const [mode, classification] of [['low_evidence', 'refusal'], ['low_confidence', 'refusal'],
  ['model_drift', 'model_drift']]) {
  test(`compiled ${mode} remains outside the comparable-action denominator`, bounded, async t => {
    const f = await compiledFixture(t, mode), result = await execute(f);
    assert.equal(result.counts.complete, 2); assert.equal(result.incomplete, true);
    assert.equal((await proofs(f)).length, 2);
    const audit = await json(join(f.manifest.output_directory, 'audit.json'));
    assert.equal(audit.counts[classification], 1); assert.equal(audit.independent_action_pairs, 0);
    if (mode === 'low_evidence') assert.equal(audit.metrics.low_evidence_rows, 2);
  });
}

test('actual missing provider usage is retained as unknown', bounded, async t => {
  const f = await compiledFixture(t, 'missing_usage'), result = await execute(f);
  assert.equal(result.counts.complete, 2); assert.equal(result.incomplete, false);
  for (const arm of ['baseline', 'tactical']) {
    assert.equal(result.by_arm[arm].input_tokens_known_sum, 0);
    assert.equal(result.by_arm[arm].input_tokens_unknown_arms, 1);
  }
});

test('compiled single-action fast path records zero actual transport invocations', bounded, async t => {
  const f = await compiledFixture(t);
  f.input.legal_action_ids = ['only'];
  f.input.observation.legal_actions = [{ action_id: 'only', action: { kind: 'end_turn' } }];
  await f.saveInput(); await f.save();
  const result = await execute(f);
  assert.equal(result.counts.complete, 2); assert.equal(result.reserved_provider_attempts, 2);
  assert.equal(result.observed_provider_attempts_known_sum, 0);
  assert.deepEqual(await proofs(f), []);
  assert.equal((await json(join(f.manifest.output_directory, 'audit.json'))).counts.forced, 1);
});

test('compiled large-catalog fallback is visible and not called a tactical comparison', bounded, async t => {
  const f = await compiledFixture(t);
  f.input.legal_action_ids = Array.from({ length: 25 }, (_, index) => `candidate-${String(index).padStart(2, '0')}`);
  await f.saveInput(); await f.save();
  const result = await execute(f), audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(result.counts.complete, 2); assert.equal(result.incomplete, true);
  assert.equal(audit.counts.tactical_fallback, 1); assert.equal(audit.independent_action_pairs, 0);
  assert.deepEqual((await proofs(f)).map(row => row.question_count), [1, 1]);
});

test('compiled bridge rejects non-ASCII action identifiers before transport invocation', bounded, async t => {
  const f = await compiledFixture(t);
  f.input.legal_action_ids = ['action-😀', 'action-\uE000'];
  await f.saveInput(); await f.save();
  const result = await execute(f);
  assert.equal(result.counts.bridge_failed, 2); assert.equal(result.incomplete, true);
  assert.equal(result.observed_provider_attempts_known_sum, 0);
  assert.deepEqual(await proofs(f), []);
  const audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(audit.counts.failed, 1); assert.equal(audit.independent_action_pairs, 0);
});

test('ten synthetic pairs reserve twenty attempts and retain twenty distinct execution identities',
  { timeout: 90000 }, async t => {
    const f = await compiledFixture(t), pair = f.manifest.pairs[0];
    f.manifest.pairs = Array.from({ length: 10 }, (_, repetition) => ({ ...pair,
      pair_id: `synthetic-${String(repetition).padStart(2, '0')}`, repetition }));
    Object.assign(f.manifest.budgets, { max_pairs: 10, max_provider_attempts: 20, total_timeout_ms: 60000 });
    await f.save();
    const result = await execute(f);
    assert.equal(result.counts.complete, 20); assert.equal(result.incomplete, false);
    assert.equal(result.reserved_provider_attempts, 20);
    assert.equal(result.observed_provider_attempts_known_sum, 20);
    assert.equal((await proofs(f)).length, 20);
    assert.equal(new Set((await captures(f)).map(row => row.record.model_execution_id_digest)).size, 20);
    const audit = await json(join(f.manifest.output_directory, 'audit-held_out.json'));
    assert.equal(audit.independent_action_pairs, 10); assert.equal(audit.counts.disagree, 10);
    assert.equal(audit.gameplay_improvement_established, false);
  });
