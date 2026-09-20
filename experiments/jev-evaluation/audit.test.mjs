// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { audit } from './audit.mjs';
import { BRIDGE_SCHEMA, PROFILE, ValidationError } from './contract.mjs';
import {
  PRIVATE_MARKER, baselineRecord, clone, hash, pairManifest, tacticalRecord,
} from './test-fixtures.mjs';

async function runAudit(baseline = baselineRecord(), tactical = tacticalRecord(), manifest = pairManifest()) {
  const records = { 'baseline.json': baseline, 'tactical.json': tactical };
  return audit(manifest, async descriptor => clone(records[descriptor.path]));
}

test('independent baseline agreement is not the tactical request baseline_choice', async () => {
  const report = await runAudit();
  assert.equal(report.analysis_status, 'complete');
  assert.equal(report.independent_action_agreement.agreements, 1);
  assert.equal(report.independent_action_agreement.action_action_denominator, 1);
  assert.equal(report.within_tactical_request_choice_agreement.agreements, 0);
  assert.equal(report.within_tactical_request_choice_agreement.is_independent_baseline, false);
  assert.equal(report.gameplay_benefit, 'unverified');
});


test('only the documented tactical state wrapper is normalized', async () => {
  const tactical = tacticalRecord();
  tactical.provider_request.state.observation_and_derived_facts = 'different source observation';
  const report = await runAudit(baselineRecord(), tactical);
  assert.equal(report.rows[0].status, 'shared_context_mismatch');
  assert.equal(report.independent_action_agreement.action_action_denominator, 0);
});

test('different action question instructions cannot masquerade as a matched input', async () => {
  const tactical = tacticalRecord();
  tactical.provider_request.questions.action.instructions = 'different objective';
  tactical.provider_request.state.tactical_context = 'different objective';
  assert.equal((await runAudit(baselineRecord(), tactical)).rows[0].status, 'shared_context_mismatch');
});

test('provider model drift is not a policy effect', async () => {
  const tactical = tacticalRecord();
  tactical.provider_response.model = 'jev-1.14.0';
  tactical.tactical.assessment.response_model = 'jev-1.14.0';
  assert.equal((await runAudit(baselineRecord(), tactical)).rows[0].status, 'model_mismatch');
});

test('inserting a tactical record as the baseline is rejected', async () => {
  const report = await runAudit(tacticalRecord(), tacticalRecord());
  assert.equal(report.rows[0].error_code, 'not_independent_baseline');
});

test('low-evidence refusal is counted without inventing an action or field-level explanation', async () => {
  const tactical = tacticalRecord();
  tactical.decision = { decision: 'reobserve', rationale: 'synthetic refusal' };
  tactical.tactical.assessment.decision = clone(tactical.decision);
  tactical.tactical.assessment.rows[1].evidence = 0.2;
  tactical.provider_response.answers.tactical_1_evidence.noul = 0.2;
  const report = await runAudit(baselineRecord(), tactical);
  assert.equal(report.diagnostic_counts.low_evidence_estimate, 1);
  assert.equal(report.rows[0].tactical_decision, 'reobserve');
  assert.equal(report.independent_action_agreement.action_action_denominator, 0);
});

for (const [name, mutate] of [
  ['unknown wrapper field', r => { r.provider_request.state.extra = 'not part of v1'; }],
  ['changed wrapper policy', r => { r.provider_request.state.question_policy = 'changed'; }],
  ['missing answer', r => { delete r.provider_response.answers.tactical_0_safety; }],
  ['missing question', r => { delete r.provider_request.questions.tactical_0_safety; }],
  ['bad utility', r => { r.tactical.assessment.rows[0].utility = 0.2; }],
  ['invalid evidence', r => { r.tactical.assessment.rows[0].evidence = 2; }],
  ['row catalog drift', r => { r.tactical.assessment.rows[0].action_id = 'not-presented'; }],
  ['different thresholds', r => { r.tactical.assessment.minimum_evidence = 0.1; }],
  ['different weights', r => { r.tactical.assessment.weights[0] = 99; }],
  ['out-of-catalog action', r => { r.decision.action_id = 'not-presented'; }],
  ['mismatched assessment decision', r => { r.tactical.assessment.decision.action_id = 'synthetic-b'; }],
  ['mismatched baseline choice field', r => { r.tactical.assessment.baseline_choice = 'synthetic-a'; }],
  ['malformed action probabilities', r => { r.provider_response.answers.action.probabilities['synthetic-a'] = 2; }],
  ['arbitrary fallback text', r => { r.tactical.applied = false; r.tactical.fallback_reason = PRIVATE_MARKER; }],
]) {
  test(`record rejects ${name}`, async () => {
    const tactical = tacticalRecord(); mutate(tactical);
    const report = await runAudit(baselineRecord(), tactical);
    assert.equal(report.rows[0].status, 'invalid_or_missing_record');
    assert.equal(report.analysis_status, 'incomplete');
    assert.equal(report.independent_action_agreement.agreements, 0);
  });
}

test('a forceable applied-tactical refusal violates the record contract', async () => {
  const tactical = tacticalRecord();
  tactical.decision = { decision: 'reobserve', candidate_action_id: 'synthetic-a' };
  tactical.tactical.assessment.decision = clone(tactical.decision);
  assert.equal((await runAudit(baselineRecord(), tactical)).rows[0].error_code, 'forceable_tactical_refusal');
});

test('fallback use is counted, not mislabeled as tactical evaluation', async () => {
  const tactical = baselineRecord();
  tactical.tactical = {
    profile: PROFILE, applied: false, fallback_reason: 'question_batch_budget', assessment: null,
  };
  const report = await runAudit(baselineRecord(), tactical);
  assert.equal(report.applied_tactical_pairs, 0);
  assert.equal(report.fallback_counts.question_batch_budget, 1);
  assert.equal(report.independent_action_agreement.agreements, 1);
});

test('forced records without input context are explicitly unbound', async () => {
  const baseline = {
    schema: BRIDGE_SCHEMA, provider_call: false, provider_request: null, provider_response: null,
    decision: { decision: 'action', action_id: 'synthetic-a' },
  };
  const tactical = { ...clone(baseline), tactical: { profile: PROFILE, applied: false, fallback_reason: 'forced_action' } };
  const report = await runAudit(baseline, tactical);
  assert.equal(report.rows[0].status, 'unbound_forced_pair');
  assert.equal(report.independent_action_agreement.action_action_denominator, 0);
});

for (const [name, mutate] of [
  ['reused execution ID', m => { m.pairs[0].tactical.execution_id = m.pairs[0].baseline.execution_id; }],
  ['different original request digest', m => { m.pairs[0].tactical.source_request_sha256 = hash('other-input'); }],
  ['different bridge binary pin', m => { m.pairs[0].tactical.bridge_sha256 = hash('other-bridge'); }],
  ['floating model pin', m => { m.model_pin = 'jev-latest'; }],
]) {
  test(`pair manifest rejects ${name}`, async () => {
    const manifest = pairManifest(); mutate(manifest);
    await assert.rejects(() => runAudit(baselineRecord(), tacticalRecord(), manifest));
  });
}

test('missing records remain scheduled and cannot become successful comparisons', async () => {
  const report = await audit(pairManifest(), async () => { throw new ValidationError('file_unavailable'); });
  assert.equal(report.scheduled_pairs, 1);
  assert.equal(report.pair_status_counts.invalid_or_missing_record, 1);
  assert.equal(report.independent_action_agreement.excluded_pairs, 1);
});

test('export contains neither raw state nor candidate descriptions', async () => {
  const report = JSON.stringify(await runAudit());
  assert.equal(report.includes(PRIVATE_MARKER), false);
  assert.equal(report.includes('Synthetic candidate'), false);
  assert.equal(report.includes('synthetic-a'), false);
  assert.equal(report.includes('baseline.json'), false);
});
