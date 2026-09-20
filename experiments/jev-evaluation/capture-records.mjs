// SPDX-License-Identifier: MIT

import { digest, exactKeys, integer, requireThat, sha256, unit } from './contract.mjs';

export const CAPTURE_SCHEMA = 'ascension.jev-redacted-capture.v1';
export const CAPTURE_PAIRS_SCHEMA = 'ascension.jev-redacted-pairs.v1';
const HEADER = ['schema', 'profile', 'bridge_digest', 'model_execution_id_digest',
  'input_digest', 'catalog_digest', 'catalog_count', 'requested_model_digest',
  'confidence_gate', 'status', 'provider_attempts', 'elapsed_ms'];
const FALLBACKS = ['forced_action', 'legacy_forced_group',
  'candidate_bound_or_incomplete_catalog', 'no_comparison_needed', 'question_batch_budget'];
const WEIGHTS = [3, 3, 2, 2, 2, 4];

// Only the model-string domain is reproduced in JavaScript. General Rust/serde_json
// request identity is an opaque producer assertion, NOT reconstructed by this reader.
export const modelFingerprint = model => sha256(
  Buffer.from(`ascension.jev-capture.v1/model\0${JSON.stringify(model)}`));

function index(value, count, nullable = false) {
  if (nullable && value === null) return;
  requireThat(integer(value) < count, 'capture_index');
}

function decision(value, count) {
  exactKeys(value, ['kind', 'selected_index', 'candidate_index']);
  requireThat(['action', 'reobserve'].includes(value.kind), 'capture_decision');
  index(value.selected_index, count, true);
  index(value.candidate_index, count, true);
  requireThat((value.kind === 'action') === (value.selected_index !== null), 'capture_decision');
  requireThat(value.kind !== 'action' || value.candidate_index === null, 'capture_decision');
}

function provider(value) {
  exactKeys(value, ['request_digest', 'shared_request_digest', 'question_set_digest',
    'response_model_digest', 'input_tokens']);
  for (const key of ['request_digest', 'shared_request_digest', 'question_set_digest']) digest(value[key]);
  if (value.response_model_digest !== null) digest(value.response_model_digest);
  if (value.input_tokens !== null) integer(value.input_tokens);
}

function tactical(value, capture) {
  requireThat(value !== null, 'capture_tactical_missing');
  requireThat(typeof value.applied === 'boolean', 'capture_tactical_applied');
  if (!value.applied) {
    exactKeys(value, ['applied', 'fallback_reason']);
    requireThat(FALLBACKS.includes(value.fallback_reason), 'capture_fallback');
    const forced = ['forced_action', 'legacy_forced_group'].includes(value.fallback_reason);
    requireThat(forced === (capture.provider_attempts === 0), 'capture_fallback');
    return;
  }
  exactKeys(value, ['applied', 'rows', 'within_request_index', 'minimum_evidence',
    'minimum_margin', 'minimum_safety']);
  requireThat(capture.provider_attempts === 1 && capture.catalog_count >= 2
    && capture.catalog_count <= 24, 'capture_applied_count');
  requireThat(value.minimum_evidence === 0.8 && value.minimum_margin === 0.1
    && value.minimum_safety === 0.5, 'capture_threshold_drift');
  index(value.within_request_index, capture.catalog_count);
  requireThat(Array.isArray(value.rows) && value.rows.length === capture.catalog_count, 'capture_rows');
  const seen = new Set();
  for (const row of value.rows) {
    exactKeys(row, ['index', 'scores', 'evidence', 'min_confidence', 'utility']);
    index(row.index, capture.catalog_count);
    requireThat(!seen.has(row.index), 'capture_duplicate_row');
    seen.add(row.index);
    requireThat(Array.isArray(row.scores) && row.scores.length === 6, 'capture_scores');
    row.scores.forEach(unit);
    unit(row.evidence); unit(row.min_confidence); unit(row.utility);
    const utility = row.scores.reduce((sum, score, i) => sum + score * WEIGHTS[i], 0) / 16;
    requireThat(Math.abs(utility - row.utility) <= 1e-9, 'capture_utility');
  }
  const sorted = [...value.rows].sort((a, b) => b.utility - a.utility || a.index - b.index);
  requireThat(sorted.every((row, i) => row.index === value.rows[i].index), 'capture_row_order');
  const first = sorted[0];
  const shouldAct = sorted.every(row => row.evidence >= value.minimum_evidence)
    && first.min_confidence >= capture.confidence_gate && first.scores[5] >= value.minimum_safety
    && first.utility - sorted[1].utility >= value.minimum_margin;
  requireThat((capture.decision.kind === 'action') === shouldAct, 'capture_selection');
  requireThat(!shouldAct || capture.decision.selected_index === first.index, 'capture_selection');
  requireThat(capture.decision.candidate_index === null, 'capture_forceable_refusal');
}

export function validateCapture(value) {
  requireThat(value !== null && typeof value === 'object', 'capture_shape');
  requireThat(['pending', 'failed', 'complete'].includes(value.status), 'capture_status');
  exactKeys(value, value.status === 'complete' ? [...HEADER, 'decision', 'provider', 'tactical'] : HEADER);
  requireThat(value.schema === CAPTURE_SCHEMA, 'capture_schema');
  requireThat(['baseline', 'jev-tactical-v1'].includes(value.profile), 'capture_profile');
  for (const key of ['bridge_digest', 'model_execution_id_digest', 'input_digest',
    'catalog_digest', 'requested_model_digest']) digest(value[key]);
  requireThat(integer(value.catalog_count) >= 1 && value.catalog_count <= 256, 'capture_catalog_count');
  unit(value.confidence_gate);
  if (value.status === 'pending') {
    requireThat(value.provider_attempts === null && value.elapsed_ms === null, 'capture_pending_metrics');
    return value;
  }
  requireThat(integer(value.provider_attempts) <= 1, 'capture_attempts');
  integer(value.elapsed_ms);
  if (value.status === 'failed') return value;
  decision(value.decision, value.catalog_count);
  requireThat((value.provider_attempts === 0) === (value.provider === null), 'capture_provider');
  if (value.provider !== null) provider(value.provider);
  else requireThat(value.decision.kind === 'action', 'capture_forced_decision');
  if (value.profile === 'baseline') requireThat(value.tactical === null, 'capture_baseline');
  else tactical(value.tactical, value);
  return value;
}