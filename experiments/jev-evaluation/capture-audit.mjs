// SPDX-License-Identifier: MIT

import { digest, exactKeys, integer, modelPin, requireThat, token } from './contract.mjs';
import { CAPTURE_PAIRS_SCHEMA, modelFingerprint, validateCapture } from './capture-records.mjs';

export function validateCaptureManifest(manifest) {
  exactKeys(manifest, ['schema', 'model', 'bridge_digest', 'pairs']);
  requireThat(manifest.schema === CAPTURE_PAIRS_SCHEMA, 'capture_pairs_schema');
  modelPin(manifest.model); digest(manifest.bridge_digest);
  requireThat(Array.isArray(manifest.pairs) && manifest.pairs.length > 0
    && manifest.pairs.length <= 4096, 'capture_pair_count');
  const ids = new Set();
  for (const pair of manifest.pairs) {
    exactKeys(pair, ['pair_id', 'baseline', 'tactical']);
    token(pair.pair_id);
    requireThat(!ids.has(pair.pair_id), 'capture_duplicate_pair');
    ids.add(pair.pair_id);
    for (const arm of ['baseline', 'tactical']) {
      const item = pair[arm];
      if (item === null) continue;
      exactKeys(item, ['path', 'sha256']);
      requireThat(typeof item.path === 'string' && item.path.length <= 240, 'record_path');
      digest(item.sha256);
    }
  }
  return manifest;
}

function classify(left, right, manifest) {
  if (left === null || right === null) return 'unreported';
  requireThat(left.profile === 'baseline' && right.profile === 'jev-tactical-v1', 'capture_arm');
  const expectedModel = modelFingerprint(manifest.model);
  for (const record of [left, right]) {
    if (record.bridge_digest !== manifest.bridge_digest
      || record.requested_model_digest !== expectedModel) return 'pin_mismatch';
  }
  if (left.status === 'pending' || right.status === 'pending') return 'pending';
  if (left.status === 'failed' || right.status === 'failed') return 'failed';
  if (left.input_digest !== right.input_digest) return 'input_mismatch';
  if (left.catalog_digest !== right.catalog_digest || left.catalog_count !== right.catalog_count) {
    return 'catalog_mismatch';
  }
  if (left.confidence_gate !== right.confidence_gate) return 'gate_mismatch';
  if (left.provider_attempts === 0 || right.provider_attempts === 0) return 'forced';
  if ([left, right].some(record => record.provider.response_model_digest === null)) {
    return 'model_identity_unverified';
  }
  if ([left, right].some(record => record.provider.response_model_digest !== expectedModel)) {
    return 'model_drift';
  }
  if (!right.tactical.applied) return 'tactical_fallback';
  if (left.provider.shared_request_digest !== right.provider.shared_request_digest) {
    return 'shared_request_mismatch';
  }
  if ([left, right].some(record => record.decision.kind !== 'action')) return 'refusal';
  return left.decision.selected_index === right.decision.selected_index ? 'agree' : 'disagree';
}

export async function auditCaptures(manifest, loader) {
  validateCaptureManifest(manifest);
  const counts = Object.fromEntries(['unreported', 'pending', 'failed', 'pin_mismatch',
    'input_mismatch', 'catalog_mismatch', 'gate_mismatch', 'forced', 'model_identity_unverified',
    'model_drift', 'tactical_fallback', 'shared_request_mismatch', 'refusal', 'agree', 'disagree']
    .map(key => [key, 0]));
  const executions = new Set();
  const outcomes = [];
  const metrics = { provider_attempts_known_sum: 0, provider_attempts_unknown_records: 0,
    input_tokens_known_sum: 0, input_tokens_unknown_records: 0,
    elapsed_ms_known_sum: 0, elapsed_ms_unknown_records: 0,
    low_evidence_rows: 0, assessed_rows: 0, within_request_action_agree: 0,
    within_request_action_pairs: 0 };
  for (const pair of manifest.pairs) {
    const records = [];
    for (const arm of ['baseline', 'tactical']) {
      const descriptor = pair[arm];
      const record = descriptor === null ? null : validateCapture(await loader(descriptor));
      records.push(record);
      if (record !== null) {
        requireThat(record.profile === (arm === 'baseline' ? 'baseline' : 'jev-tactical-v1'), 'capture_arm');
        requireThat(!executions.has(record.model_execution_id_digest), 'capture_reused_execution');
        executions.add(record.model_execution_id_digest);
      }
      measurements(record, metrics);
    }
    const status = classify(...records, manifest);
    counts[status] += 1;
    outcomes.push({ pair_position: outcomes.length, status });
  }
  const denominator = counts.agree + counts.disagree;
  return {
    schema: 'ascension.jev-redacted-audit.v1', evidence: 'producer_assertions_not_native_witnesses',
    scheduled_pairs: manifest.pairs.length, counts, outcomes,
    independent_action_pairs: denominator,
    independent_action_agreement: denominator === 0 ? null : counts.agree / denominator,
    metrics,
    incomplete: Object.entries(counts).some(([key, count]) => !['agree', 'disagree'].includes(key) && count > 0),
    gameplay_improvement_established: false,
  };
}

function measurements(record, metrics) {
  for (const [name, value] of [
    ['provider_attempts', record?.provider_attempts ?? null],
    ['elapsed_ms', record?.elapsed_ms ?? null],
    ['input_tokens', record?.status === 'complete' && record.provider_attempts === 0
      ? 0 : record?.provider?.input_tokens ?? null],
  ]) {
    if (value === null) metrics[`${name}_unknown_records`] += 1;
    else metrics[`${name}_known_sum`] = integer(metrics[`${name}_known_sum`] + value);
  }
  if (record?.status !== 'complete' || record.tactical?.applied !== true) return;
  metrics.assessed_rows += record.tactical.rows.length;
  metrics.low_evidence_rows += record.tactical.rows.filter(
    row => row.evidence < record.tactical.minimum_evidence).length;
  if (record.decision.kind === 'action') {
    metrics.within_request_action_pairs += 1;
    metrics.within_request_action_agree += Number(
      record.decision.selected_index === record.tactical.within_request_index);
  }
}