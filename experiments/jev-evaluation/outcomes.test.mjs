// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { compare } from './outcomes.mjs';
import { ARMS, OUTCOMES } from './contract.mjs';
import { clone, cohort, hash, result } from './test-fixtures.mjs';

test('missing runs remain in scheduled denominators and are not called defeats', () => {
  const c = cohort();
  const row = result(c, c.pairs[1], 'baseline', 'victory');
  const report = compare(c, hash('manifest'), [row]);
  assert.equal(report.unreported_results, 3);
  assert.equal(report.analysis_status, 'incomplete');
  const tactical = report.splits.held_out.arms.tactical;
  assert.equal(tactical.scheduled, 1);
  assert.equal(tactical.outcomes.unreported, 1);
  assert.equal(tactical.outcomes.defeat, 0);
  assert.equal(tactical.start_status_unknown, 1);
  assert.equal(report.splits.held_out.paired.operational_victory_difference_reported_pairs, null);
});


test('complete synthetic evidence never establishes gameplay improvement', () => {
  const c = cohort();
  const rows = c.pairs.flatMap(pair => ARMS.map(arm => result(c, pair, arm, arm === 'tactical' ? 'victory' : 'defeat')));
  const report = compare(c, hash('manifest'), rows);
  assert.equal(report.analysis_status, 'complete');
  assert.equal(report.promotion_status, 'blocked_synthetic_evidence');
  assert.equal(report.splits.held_out.paired.terminal_only_victory_difference, 1);
});

for (const outcome of OUTCOMES) {
  test(`outcome ${outcome} retains its own category`, () => {
    const c = cohort();
    const report = compare(c, hash('manifest'), [result(c, c.pairs[0], 'baseline', outcome)]);
    assert.equal(report.splits.calibration.arms.baseline.outcomes[outcome], 1);
  });
}

test('partial measurement totals remain unknown instead of silently becoming zero', () => {
  const c = cohort();
  c.pairs[0].split = 'held_out';
  const row = result(c, c.pairs[0], 'baseline'); row.provider_calls = 10;
  const report = compare(c, hash('manifest'), [row]);
  const counts = report.splits.held_out.arms.baseline.metrics.provider_calls;
  assert.deepEqual(counts, { observed_total: 10, known_count: 1, unknown_count: 1, complete_total: null });
  assert.equal(report.splits.held_out.arms.tactical.metrics.provider_calls.observed_total, null);
});

test('terminal-only comparisons expose attrition bias', () => {
  const c = cohort();
  const rows = [result(c, c.pairs[1], 'baseline', 'timeout'), result(c, c.pairs[1], 'tactical', 'victory')];
  const paired = compare(c, hash('manifest'), rows).splits.held_out.paired;
  assert.equal(paired.operational_victory_difference_reported_pairs, 1);
  assert.equal(paired.terminal_only_victory_difference, null);
  assert.equal(paired.terminal_only_is_selection_biased, true);
});

test('equal-seed weighting avoids giving repeated seeds extra weight', () => {
  const c = cohort();
  c.pairs = [
    { ...c.pairs[1], pair_id: 'p1' },
    { ...c.pairs[1], pair_id: 'p2', repetition: 1 },
    { ...c.pairs[1], pair_id: 'p3', seed_sha256: hash('other-seed') },
  ];
  const rows = c.pairs.flatMap((pair, i) => [
    result(c, pair, 'baseline', i < 2 ? 'defeat' : 'victory'),
    result(c, pair, 'tactical', i < 2 ? 'victory' : 'defeat'),
  ]);
  const paired = compare(c, hash('manifest'), rows).splits.held_out.paired;
  assert.equal(paired.distinct_reported_seed_clusters, 2);
  assert.equal(paired.operational_victory_difference_reported_pairs, 1 / 3);
  assert.equal(paired.operational_victory_difference_equal_seed_weight, 0);
});

for (const [name, mutate] of [
  ['manifest mutation', r => { r.manifest_sha256 = hash('other'); }],
  ['model drift', r => { r.pins.model = 'jev-1.14.0'; }],
  ['mixed synthetic/live label', r => { r.evidence_kind = 'operator_recorded'; }],
  ['unplanned pair', r => { r.pair_id = 'not-planned'; }],
  ['wrong arm', r => { r.arm = 'other'; }],
  ['order violation', r => { r.slot = 1 - r.slot; }],
  ['missing terminal witness', r => { r.terminal_witness_sha256 = null; }],
  ['negative cost', r => { r.cost_micro_usd = -1; }],
  ['fractional calls', r => { r.provider_calls = 1.5; }],
  ['invalid start status', r => { r.started = false; }],
  ['unknown field', r => { r.raw_prompt = 'private'; }],
]) {
  test(`result rejects ${name}`, () => {
    const c = cohort(); const row = result(c, c.pairs[0], 'baseline'); mutate(row);
    assert.throws(() => compare(c, hash('manifest'), [row]));
  });
}

test('duplicate arm reports cannot be cherry-picked', () => {
  const c = cohort(); const row = result(c, c.pairs[0], 'baseline');
  const other = clone(row); other.run_id = 'run:other'; other.episode_id = 'episode:other';
  assert.throws(() => compare(c, hash('manifest'), [row, other]), /duplicate_arm_result/);
});

test('a reused episode identity cannot masquerade as an independent run', () => {
  const c = cohort(); const a = result(c, c.pairs[0], 'baseline'); const b = result(c, c.pairs[0], 'tactical');
  b.episode_id = a.episode_id;
  assert.throws(() => compare(c, hash('manifest'), [a, b]), /duplicate_execution_identity/);
});

test('completed operator records still require independent review, never automatic promotion', () => {
  const c = cohort(); c.evidence_kind = 'operator_recorded';
  const rows = c.pairs.flatMap(pair => ARMS.map(arm => result(c, pair, arm)));
  assert.equal(compare(c, hash('manifest'), rows).promotion_status,
    'requires_independent_runtime_and_statistical_review');
});
