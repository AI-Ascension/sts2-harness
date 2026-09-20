// SPDX-License-Identifier: MIT

// Narrow, read-only pilot admission. It neither authors inputs nor changes a decision policy.
import { canonical, requireThat, sha256 } from './contract.mjs';
import { MAX_INPUT, RUN_SCHEMA, schedule, validateRunner } from './runner-contract.mjs';

export const PILOT_PROFILE = 'jev-frozen-pilot-v1';
export const PILOT_PAIRS = 10;

export function validatePilot(manifest) {
  validateRunner(manifest);
  const b = manifest.budgets;
  requireThat(manifest.pairs.length === PILOT_PAIRS && b.max_pairs === PILOT_PAIRS
    && b.max_provider_attempts === 2 * PILOT_PAIRS, 'pilot_ten_pairs_twenty_attempts');
  requireThat(b.total_timeout_ms <= 1800000 && b.max_total_input_bytes <= PILOT_PAIRS * MAX_INPUT,
    'pilot_budget');
  return manifest;
}

export function expectedPilotPlan(manifest, hash) {
  validatePilot(manifest);
  return { schema: RUN_SCHEMA, manifest_sha256: hash, source_revision: manifest.source_revision,
    bridge_digest: manifest.bridge.sha256, transport_digest: manifest.transport.sha256,
    evidence_kind: manifest.evidence_kind, budgets: manifest.budgets,
    scheduled: schedule(manifest, hash) };
}

export function cohortDescription(pairs) {
  return { scheduled_pairs: pairs.length,
    declared_clusters: new Set(pairs.map(pair => pair.cluster_sha256)).size,
    distinct_input_file_hashes: new Set(pairs.map(pair => pair.input_sha256)).size,
    nonzero_repetition_pairs: pairs.filter(pair => pair.repetition !== 0).length };
}

export function inputDescription(inputs) {
  const semantic = new Set();
  const counts = { single_action: 0, two_to_twenty_four: 0, over_twenty_four: 0 };
  for (const input of inputs) {
    // The compiled bridge in #377 refuses non-ASCII IDs before transport. Never silently drop them.
    requireThat(input.legal_action_ids.every(id => /^[\x20-\x7e]+$/.test(id)),
      'pilot_non_ascii_action_id');
    const value = { ...input }; delete value.model_execution_id;
    semantic.add(sha256(canonical(value)));
    const size = input.legal_action_ids.length;
    counts[size === 1 ? 'single_action' : size <= 24 ? 'two_to_twenty_four' : 'over_twenty_four']++;
  }
  return { distinct_inputs_excluding_execution_id: semantic.size, raw_catalog_size_counts: counts,
    catalog_size_does_not_establish_tactical_applicability: true };
}
