// SPDX-License-Identifier: MIT
// Hand-authored synthetic MIT fixtures. No game/provider records or proprietary data.

import {
  BASE_REVISION, BRIDGE_SCHEMA, COHORT_SCHEMA, PAIRS_SCHEMA, PROFILE,
  RESULT_SCHEMA, firstArm, sha256,
} from './contract.mjs';
import { QUESTION_POLICY, TACTICAL_LIMITS } from './records.mjs';

export const clone = value => structuredClone(value);
export const hash = label => sha256(`synthetic-only:${label}`);
export const MODEL = 'jev-1.13.0';
export const PRIVATE_MARKER = 'DO-NOT-EXPORT-RAW-STATE';

export function cohort() {
  return {
    schema: COHORT_SCHEMA, experiment_id: 'synthetic-demo', evidence_kind: 'synthetic',
    pins: {
      source_revision: BASE_REVISION, model: MODEL, game_build_id: 'synthetic-no-game',
      bridge_sha256: hash('bridge'), host_mod_sha256: hash('mod'),
      protocol_sha256: hash('protocol'), observation_policy_sha256: hash('observation'),
      budget_sha256: hash('budget'), baseline_policy_sha256: hash('baseline'),
      tactical_policy_sha256: hash('tactical'),
    },
    pairs: [
      { pair_id: 'pair-calibration', seed_sha256: hash('seed-calibration'), repetition: 0, split: 'calibration' },
      { pair_id: 'pair-held-out', seed_sha256: hash('seed-held-out'), repetition: 0, split: 'held_out' },
    ],
  };
}


export function result(plan, pair, arm, outcome = 'defeat', manifestDigest = hash('manifest')) {
  return {
    schema: RESULT_SCHEMA, manifest_sha256: manifestDigest, pair_id: pair.pair_id, arm,
    slot: arm === firstArm(plan, pair) ? 0 : 1, pins: clone(plan.pins),
    evidence_kind: plan.evidence_kind, run_id: `run:${pair.pair_id}:${arm}`,
    episode_id: `episode:${pair.pair_id}:${arm}`, started: outcome !== 'not_started', outcome,
    terminal_witness_sha256: ['victory', 'defeat'].includes(outcome) ? hash(`witness:${pair.pair_id}:${arm}`) : null,
    provider_calls: null, input_tokens: null, latency_ms: null,
    cost_micro_usd: null, combat_hp_lost: null,
  };
}

export function baselineRecord() {
  return {
    schema: BRIDGE_SCHEMA, provider_call: true,
    provider_request: {
      model: MODEL, state: JSON.stringify({ fixture: PRIVATE_MARKER, generation: 1 }),
      questions: {
        action: {
          type: 'choice', instructions: 'Objective: synthetic offline test only',
          criteria: { 'synthetic-a': 'Synthetic candidate A', 'synthetic-b': 'Synthetic candidate B' },
        },
      },
    },
    provider_response: {
      model: MODEL,
      answers: {
        action: {
          type: 'choice', choice: 'synthetic-a', confidence: 0.9,
          probabilities: { 'synthetic-a': 0.9, 'synthetic-b': 0.1 },
        },
      },
      usage: { input_tokens: 100, output_tokens: 0 },
    },
    decision: {
      decision: 'action', action_id: 'synthetic-a', confidence: 90,
      rationale: 'bridge-authored synthetic evidence',
    },
  };
}

export function tacticalRecord() {
  const record = baselineRecord();
  const body = record.provider_request;
  body.state = {
    observation_and_derived_facts: body.state,
    tactical_context: body.questions.action.instructions,
    tactical_limits: TACTICAL_LIMITS, evaluation_profile: PROFILE, question_policy: QUESTION_POLICY,
  };
  const axes = ['immediate', 'threat', 'setup', 'resource', 'strategy', 'safety'];
  const levels = ['Poor contribution', 'Mixed or neutral contribution', 'Strong contribution'];
  for (const [index, id] of Object.keys(body.questions.action.criteria).entries()) {
    for (const axis of axes) {
      const key = `tactical_${index}_${axis}`;
      body.questions[key] = {
        type: 'score', criteria: levels,
        instructions: { candidate_id: id, candidate_description: 'Synthetic', question: 'Synthetic?' },
      };
      const good = index === 0 ? 0.9 : 0.1;
      record.provider_response.answers[key] = {
        type: 'score', score: good * 2, confidence: 0.9,
        probabilities: { '0': 1 - good, '1': 0, '2': good },
        legend: Object.fromEntries(levels.map((level, i) => [String(i), level])),
      };
    }
    body.questions[`tactical_${index}_evidence`] = {
      type: 'noul', instructions: 'Synthetic evidence question',
      criteria: { true: 'Sufficient admitted evidence', false: 'Important facts are missing' },
    };
    record.provider_response.answers[`tactical_${index}_evidence`] = { type: 'noul', noul: 0.95 };
  }
  // Intentionally differ from the separately captured baseline to test both denominators.
  record.provider_response.answers.action.choice = 'synthetic-b';
  record.provider_response.answers.action.probabilities = { 'synthetic-a': 0.1, 'synthetic-b': 0.9 };
  record.tactical = {
    profile: PROFILE, applied: true, fallback_reason: null,
    question_set_digest: hash('not-a-real-rust-question-digest'), request_digest: hash('not-a-real-rust-request-digest'),
    assessment: {
      profile: PROFILE, decision: clone(record.decision),
      rows: [
        { action_id: 'synthetic-a', scores: Array(6).fill(0.9), min_confidence: 0.9, evidence: 0.95, utility: 0.9 },
        { action_id: 'synthetic-b', scores: Array(6).fill(0.1), min_confidence: 0.9, evidence: 0.95, utility: 0.1 },
      ],
      axes, weights: [3, 3, 2, 2, 2, 4], minimum_evidence: 0.8, minimum_margin: 0.1,
      minimum_safety: 0.5, confidence_gate: 0.2, baseline_choice: 'synthetic-b', response_model: MODEL,
    },
  };
  return record;
}

export function pairManifest() {
  const descriptor = arm => ({
    execution_id: `model-execution:synthetic:${arm}`, source_request_sha256: hash('input'),
    bridge_sha256: hash('bridge'), path: `${arm}.json`, sha256: hash(`record:${arm}`),
  });
  return {
    schema: PAIRS_SCHEMA, evidence_kind: 'synthetic', model_pin: MODEL,
    bridge_sha256: hash('bridge'),
    pairs: [{ case_id: 'synthetic-case', baseline: descriptor('baseline'), tactical: descriptor('tactical') }],
  };
}
