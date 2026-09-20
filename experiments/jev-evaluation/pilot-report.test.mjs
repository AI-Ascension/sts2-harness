// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { reportPilot } from './pilot.mjs';
import { measurements } from './pilot-analysis.mjs';
import { inspectRun, readRun, summarize } from './runner-journal.mjs';
import { slotName } from './runner-evidence.mjs';
import { json, pilotFixture, rewriteCapture, syntheticRun, writeJson } from './pilot-test-fixtures.mjs';

const unix = { skip: process.platform === 'win32' };

test('diagnostics re-audit captures and retain legacy inspection output', unix, async t => {
  const f = await pilotFixture(t), original = await readFile(f.path);
  const { audit } = await syntheticRun(f), report = await reportPilot(f.path);
  assert.equal(report.incomplete, false); assert.deepEqual(report.overall.audit, audit);
  assert.equal(report.overall.audit.independent_action_pairs, 10);
  assert.equal(report.overall.audit.counts.disagree, 10);
  assert.equal(report.overall.audit.metrics.within_request_action_pairs, 10);
  assert.equal(report.overall.paired_measurements.metrics.input_tokens.mean_known, 100);
  assert.equal(report.overall.paired_measurements.metrics.elapsed_ms.mean_known, 200);
  assert.equal(report.splits.calibration.overall, undefined);
  assert.equal(report.splits.calibration.audit.scheduled_pairs, 5);
  assert.equal(report.splits.held_out.audit.scheduled_pairs, 5);
  assert.equal(report.execution_performed, false); assert.equal(report.policy_changed, false);
  assert.equal(report.gameplay_improvement_established, false);
  const { plan, rows } = await readRun(f.manifest.output_directory);
  assert.deepEqual(await inspectRun(f.manifest.output_directory), {
    ...summarize(plan, rows, 'inspection_not_liveness_proof'), pair_audit_performed: false });
  assert.deepEqual(await readFile(f.path), original);
  for (const value of [f.root, 'PRIVATE_OBJECTIVE', 'private-state', 'private-action', 'private-pair',
    'TYPESAFE_API_KEY', 'private-original']) assert.equal(JSON.stringify(report).includes(value), false);
});

for (const [mode, reason] of [['low_evidence', 'low_evidence_decisions'],
  ['low_confidence', 'low_confidence_decisions'], ['low_safety', 'low_safety_decisions'],
  ['low_margin', 'low_margin_decisions']]) {
  test(`${mode} is diagnosed without discarding baseline or refusal costs`, unix, async t => {
    const f = await pilotFixture(t, [mode]); await syntheticRun(f);
    const report = await reportPilot(f.path), tactical = report.overall.by_arm.tactical;
    assert.equal(report.incomplete, true);
    assert.equal(report.overall.audit.counts.refusal, 1);
    assert.equal(report.overall.audit.independent_action_pairs, 9);
    assert.equal(tactical.tactical_gates[reason], 1);
    assert.equal(tactical.refusals_over_scheduled.denominator, 10);
    assert.equal(tactical.refusals_over_complete_captures.numerator, 1);
    assert.equal(report.overall.by_arm.baseline.actions_over_scheduled.numerator, 10);
    assert.equal(report.overall.paired_measurements.eligible_pairs, 10);
    assert.equal(report.overall.paired_measurements.metrics.input_tokens.known_count, 10);
  });
}

test('missing usage stays unknown and does not create a zero-cost tactical arm', unix, async t => {
  const f = await pilotFixture(t, ['missing_usage']); await syntheticRun(f);
  const report = await reportPilot(f.path);
  assert.equal(report.overall.by_arm.tactical.measurements.input_tokens.unknown_count, 1);
  assert.equal(report.overall.by_arm.tactical.measurements.input_tokens.known_sum, 1800);
  assert.equal(report.overall.paired_measurements.metrics.input_tokens.unknown_count, 1);
  assert.equal(report.overall.paired_measurements.metrics.input_tokens.mean_known, 100);
});

test('forced, fallback, drift and failed pairs remain in their own denominators', unix, async t => {
  const f = await pilotFixture(t, ['forced', 'fallback', 'model_drift', 'failed']);
  await syntheticRun(f); const report = await reportPilot(f.path), audit = report.overall.audit;
  for (const status of ['forced', 'tactical_fallback', 'model_drift', 'failed']) assert.equal(audit.counts[status], 1);
  assert.equal(audit.independent_action_pairs, 6);
  assert.equal(report.overall.paired_measurements.eligible_pairs, 6);
  assert.equal(report.overall.by_arm.tactical.forced_actions, 1);
  assert.equal(report.overall.by_arm.tactical.tactical_fallback_reasons.candidate_bound_or_incomplete_catalog, 1);
  assert.equal(report.overall.execution.observed_provider_attempts_known_sum, 18);
  assert.equal(report.overall.by_arm.baseline.captures_missing_or_incomplete, 1);
});

test('missing final summary and outstanding reservations remain incomplete, never resumed', unix, async t => {
  const f = await pilotFixture(t, ['interrupted_unknown', 'not_started', 'pending']);
  await syntheticRun(f, { finalize: false }); const before = await readdir(f.manifest.output_directory);
  const report = await reportPilot(f.path);
  assert.equal(report.finalized_record_present, false); assert.equal(report.incomplete, true);
  assert.equal(report.overall.execution.counts.interrupted_unknown, 2);
  assert.equal(report.overall.execution.counts.not_started, 2);
  assert.equal(report.overall.execution.observed_provider_attempts_unknown_arms, 4);
  assert.deepEqual(await readdir(f.manifest.output_directory), before);
});

test('zero comparable decisions return null agreement, not perfect agreement', unix, async t => {
  const f = await pilotFixture(t, Array(10).fill('failed')); await syntheticRun(f);
  const report = await reportPilot(f.path);
  assert.equal(report.overall.audit.independent_action_agreement, null);
  assert.equal(report.overall.by_arm.tactical.refusals_over_complete_captures.value, null);
  assert.equal(report.overall.paired_measurements.metrics.input_tokens.mean_known, null);
});

test('cached audits are not trusted or executed, and original inputs are not reopened', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  await writeFile(join(f.manifest.output_directory, 'audit.json'), '{UNTRUSTED_CACHE');
  await Promise.all(f.manifest.pairs.map(pair => rm(join(f.root, pair.input_path))));
  await rm(f.manifest.bridge.path); await rm(f.manifest.transport.path);
  const report = await reportPilot(f.path);
  assert.equal(report.incomplete, false); assert.equal(report.overall.audit.counts.disagree, 10);
});

for (const [name, change] of [['budget', m => m.budgets.total_timeout_ms++],
  ['source revision', m => m.source_revision = 'a'.repeat(40)],
  ['execution order identity', m => m.experiment_id = 'different']]) {
  test(`post-run ${name} manifest edits cannot be rebound to old results`, unix, async t => {
    const f = await pilotFixture(t); await syntheticRun(f); change(f.manifest); await f.save();
    await assert.rejects(reportPilot(f.path), { code: 'pilot_frozen_plan_mismatch' });
  });
}

test('capture content digest mismatch is refused instead of silently disappearing', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  await writeFile(join(f.manifest.output_directory, f.rows[0].capture.path), '{}');
  await assert.rejects(reportPilot(f.path), { code: 'pilot_capture_digest' });
});

for (const key of ['model_execution_id_digest', 'bridge_digest', 'requested_model_digest']) {
  test(`a rewritten ${key} cannot be admitted even with an updated file digest`, unix, async t => {
    const f = await pilotFixture(t); await syntheticRun(f);
    await rewriteCapture(f, 0, record => record[key] = 'f'.repeat(64));
    await assert.rejects(reportPilot(f.path), { code: 'pilot_capture_identity' });
  });
}

test('journal metrics must match their pinned capture', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  f.rows[0].input_tokens++;
  await writeJson(join(f.manifest.output_directory, `${slotName(0)}.result.json`), f.rows[0]);
  await assert.rejects(reportPilot(f.path), { code: 'pilot_capture_measurement' });
});

test('a complete capture from a failed process remains quarantined', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  f.rows[0].status = 'bridge_failed'; f.rows[0].process_status = 'bridge_failed';
  await writeJson(join(f.manifest.output_directory, `${slotName(0)}.result.json`), f.rows[0]);
  await assert.rejects(reportPilot(f.path), { code: 'pilot_quarantined_capture' });
});

test('complete execution without a pinned capture is not accepted', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f); f.rows[0].capture = null;
  await writeJson(join(f.manifest.output_directory, `${slotName(0)}.result.json`), f.rows[0]);
  await assert.rejects(reportPilot(f.path), { code: 'pilot_complete_without_capture' });
});

test('tampered final summaries and corrupt journals are errors, not missing records', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  const finalPath = join(f.manifest.output_directory, 'run.result.json'), value = await json(finalPath);
  value.counts.complete = 999; await writeJson(finalPath, value);
  await assert.rejects(reportPilot(f.path), { code: 'pilot_final_summary_mismatch' });
  await writeFile(join(f.manifest.output_directory, `${slotName(0)}.result.json`), '{bad');
  await assert.rejects(reportPilot(f.path), { code: 'invalid_json' });
});

test('non-private files and symlinked captures are rejected', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  const path = join(f.manifest.output_directory, f.rows[0].capture.path);
  await chmod(path, 0o644); await assert.rejects(reportPilot(f.path));
  await chmod(path, 0o600);
  const bytes = await readFile(path), target = join(f.root, 'replacement.json');
  await writeFile(target, bytes, { mode: 0o600 }); await rm(path); await symlink(target, path);
  await assert.rejects(reportPilot(f.path), { code: 'runner_symlink' });
});

test('descriptive measurement statistics preserve unknowns and signed differences', () => {
  assert.deepEqual(measurements([null, -10, 10], true), { known_count: 2, unknown_count: 1,
    known_sum: 0, mean_known: 0, median_known: 0, minimum_known: -10, maximum_known: 10 });
  assert.equal(measurements([]).mean_known, null);
});

test('multiple refusal gates overlap without becoming multiple refused decisions', unix, async t => {
  const f = await pilotFixture(t, ['low_safety']); await syntheticRun(f);
  const slot = f.records.findIndex(record => record.profile === 'jev-tactical-v1');
  await rewriteCapture(f, slot, record => {
    record.tactical.rows[0].min_confidence = 0.1;
    record.tactical.rows[1].evidence = 0.1;
  });
  const report = await reportPilot(f.path), gates = report.overall.by_arm.tactical.tactical_gates;
  assert.equal(gates.refused_decisions, 1); assert.equal(gates.low_evidence_decisions, 1);
  assert.equal(gates.low_confidence_decisions, 1); assert.equal(gates.low_safety_decisions, 1);
  assert.equal(gates.reasons_can_overlap, true);
});

test('report snapshot identity is stable across read-only invocations', unix, async t => {
  const f = await pilotFixture(t); await syntheticRun(f);
  const first = await reportPilot(f.path), second = await reportPilot(f.path);
  assert.match(first.evidence_snapshot_sha256, /^[0-9a-f]{64}$/);
  assert.deepEqual(second, first);
});

test('CLI emits valid incomplete diagnostics with exit three rather than suppressing failures', unix, async t => {
  const f = await pilotFixture(t, ['failed']); await syntheticRun(f);
  const cli = fileURLToPath(new URL('./pilot-cli.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [cli, 'report', f.path], { encoding: 'utf8' });
  assert.equal(result.status, 3); assert.equal(result.stderr, '');
  assert.equal(JSON.parse(result.stdout).overall.audit.counts.failed, 1);
  assert.equal(result.stdout.includes(f.root), false);
});
