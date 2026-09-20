// SPDX-License-Identifier: MIT

import {
  ARMS, MAX_PAIRS, PAIRS_SCHEMA, ValidationError, canonical, digest, evidenceKind,
  exactKeys, modelPin, requireThat, token,
} from './contract.mjs';
import { inspectRecord } from './records.mjs';

export function validatePairs(manifest) {
  exactKeys(manifest, ['schema', 'evidence_kind', 'model_pin', 'bridge_sha256', 'pairs']);
  requireThat(manifest.schema === PAIRS_SCHEMA, 'decision_pairs_schema');
  evidenceKind(manifest.evidence_kind);
  modelPin(manifest.model_pin);
  digest(manifest.bridge_sha256);
  requireThat(Array.isArray(manifest.pairs) && manifest.pairs.length > 0
    && manifest.pairs.length <= MAX_PAIRS, 'pair_count_bound');
  const cases = new Set();
  const executions = new Set();
  for (const pair of manifest.pairs) {
    exactKeys(pair, ['case_id', ...ARMS]);
    token(pair.case_id);
    requireThat(!cases.has(pair.case_id), 'duplicate_case');
    cases.add(pair.case_id);
    for (const arm of ARMS) {
      const descriptor = pair[arm];
      exactKeys(descriptor, ['execution_id', 'source_request_sha256', 'bridge_sha256', 'path', 'sha256']);
      token(descriptor.execution_id);
      digest(descriptor.source_request_sha256);
      digest(descriptor.sha256);
      requireThat(descriptor.bridge_sha256 === manifest.bridge_sha256, 'bridge_pin_mismatch');
      requireThat(typeof descriptor.path === 'string', 'record_path');
      requireThat(!executions.has(descriptor.execution_id), 'reused_execution_identity');
      executions.add(descriptor.execution_id);
    }
    requireThat(pair.baseline.source_request_sha256 === pair.tactical.source_request_sha256,
      'source_input_identity_mismatch');
  }
  return manifest;
}


async function auditPair(manifest, pair, load) {
  const baseline = inspectRecord(await load(pair.baseline), 'baseline');
  const tactical = inspectRecord(await load(pair.tactical), 'tactical');
  const row = {
    case_id: pair.case_id, status: 'comparable', applied: tactical.applied,
    fallback_reason: tactical.fallback_reason,
    baseline_provider_call: baseline.provider_call, tactical_provider_call: tactical.provider_call,
    diagnostics: tactical.diagnostics ?? null,
  };
  if (!baseline.provider_call || !tactical.provider_call) {
    return { ...row, status: 'unbound_forced_pair' };
  }
  if ([baseline, tactical].some(value => value.requested_model !== manifest.model_pin
    || value.response_model !== manifest.model_pin)) return { ...row, status: 'model_mismatch' };
  if (canonical(baseline.context) !== canonical(tactical.context)) {
    return { ...row, status: 'shared_context_mismatch' };
  }
  return {
    ...row, baseline_decision: baseline.decision_kind, tactical_decision: tactical.decision_kind,
    independent_action_agreement: baseline.action_id !== null && tactical.action_id !== null
      ? baseline.action_id === tactical.action_id : null,
  };
}

export async function audit(manifest, load) {
  validatePairs(manifest);
  const rows = [];
  for (const pair of manifest.pairs) {
    try {
      rows.push(await auditPair(manifest, pair, load));
    } catch (error) {
      if (!(error instanceof ValidationError)) throw error;
      rows.push({ case_id: pair.case_id, status: 'invalid_or_missing_record', error_code: error.code });
    }
  }
  const comparable = rows.filter(row => row.status === 'comparable');
  const compared = comparable.filter(row => typeof row.independent_action_agreement === 'boolean');
  const within = comparable.filter(row => typeof row.diagnostics?.within_request_choice_agrees === 'boolean');
  const counts = {};
  const fallbacks = {};
  for (const row of rows) {
    counts[row.status] = (counts[row.status] ?? 0) + 1;
    if (row.fallback_reason) fallbacks[row.fallback_reason] = (fallbacks[row.fallback_reason] ?? 0) + 1;
  }
  return {
    schema: 'ascension.jev-decision-audit.v1', evidence_kind: manifest.evidence_kind,
    evidence_status: 'source-derived',
    analysis_status: comparable.length === rows.length ? 'complete' : 'incomplete',
    gameplay_benefit: 'unverified', scheduled_pairs: rows.length, pair_status_counts: counts,
    applied_tactical_pairs: comparable.filter(row => row.applied).length,
    fallback_counts: fallbacks,
    independent_action_agreement: {
      agreements: compared.filter(row => row.independent_action_agreement).length,
      action_action_denominator: compared.length,
      excluded_pairs: rows.length - compared.length,
    },
    within_tactical_request_choice_agreement: {
      agreements: within.filter(row => row.diagnostics.within_request_choice_agrees).length,
      action_denominator: within.length, is_independent_baseline: false,
    },
    diagnostic_counts: Object.fromEntries([
      'low_evidence_estimate', 'below_confidence_gate', 'below_safety_gate', 'below_margin_gate',
    ].map(key => [key, comparable.filter(row => row.diagnostics?.[key]).length])),
    caveats: [
      'Only the documented tactical state wrapper is normalized; all shared state and action-question content must match.',
      'Execution IDs and bridge pins are operator declarations, not cryptographic proof of independent execution.',
      'This is a record-shape and consistency audit, not a re-execution of the Rust provider validator or selector.',
      'Low evidence is a model estimate; it does not establish which authoritative game fields are absent.',
      'File hashes verify exact imported bytes. Rust request/question digests are not recomputed using JavaScript serialization.',
      'Forced records carry no provider context and are reported as unbound, not silently counted as agreement.',
    ], rows,
  };
}
