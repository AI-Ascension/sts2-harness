// SPDX-License-Identifier: MIT

// Read-only preflight and result analysis. No process spawn, provider call, game action or write.
import { dirname, join, resolve } from 'node:path';
import { canonical, parseJson, requireThat, sha256 } from './contract.mjs';
import { CAPTURE_PAIRS_SCHEMA, modelFingerprint, validateCapture } from './capture-records.mjs';
import { auditCaptures } from './capture-audit.mjs';
import { preflight, privateBytes, privateDirectory, readOptionalJson } from './runner-io.mjs';
import { readRun, summarize } from './runner-journal.mjs';
import { slotName } from './runner-evidence.mjs';
import { PILOT_PROFILE, cohortDescription, expectedPilotPlan, inputDescription, validatePilot } from './pilot-profile.mjs';
import { diagnosticGroup } from './pilot-analysis.mjs';

async function pilotManifest(path) {
  const absolute = resolve(path);
  await privateDirectory(dirname(absolute));
  const bytes = await privateBytes(absolute, 1024 * 1024);
  try { return { manifest: validatePilot(parseJson(bytes)), hash: sha256(bytes) }; }
  finally { bytes.fill(0); }
}

export async function planPilot(path) {
  // Refuse an over-budget profile before the general runner reads any original inputs.
  const frozen = await pilotManifest(path), prepared = await preflight(path);
  try {
    requireThat(prepared.hash === frozen.hash, 'pilot_manifest_changed');
    return { schema: 'ascension.jev-pilot-plan.v1', profile: PILOT_PROFILE,
      manifest_sha256: frozen.hash, evidence_kind: frozen.manifest.evidence_kind,
      source_revision: frozen.manifest.source_revision, source_revision_is_operator_assertion: true,
      cohort: cohortDescription(frozen.manifest.pairs), inputs: inputDescription(prepared.inputs),
      splits: Object.fromEntries(['calibration', 'held_out'].map(split => [split,
        cohortDescription(frozen.manifest.pairs.filter(pair => pair.split === split))])),
      budgets: frozen.manifest.budgets, scheduled_arms: 20, local_preflight_passed: true,
      execution_performed: false, execution_requires_exact_manifest_hash_approval: true,
      provider_availability_verified: false, native_outcomes_verified: false,
      policy_changed: false, hard_token_or_money_budget_enforced: false };
  } finally { prepared.inputs.length = 0; }
}

function bindCapture(record, manifest, entry, row) {
  validateCapture(record);
  requireThat(record.profile === (entry.arm === 'baseline' ? 'baseline' : 'jev-tactical-v1')
    && record.model_execution_id_digest === entry.execution_id_digest
    && record.bridge_digest === manifest.bridge.sha256
    && record.requested_model_digest === modelFingerprint(manifest.model)
    && record.confidence_gate === manifest.confidence_gate_percent / 100, 'pilot_capture_identity');
  requireThat(record.provider_attempts === row.observed_provider_attempts
    && record.elapsed_ms === row.capture_elapsed_ms, 'pilot_capture_measurement');
  const tokens = record.status !== 'complete' ? null
    : record.provider_attempts === 0 ? 0 : record.provider.input_tokens;
  requireThat(tokens === row.input_tokens, 'pilot_capture_measurement');
  requireThat((record.status === 'complete') === (row.status === 'complete'), 'pilot_quarantined_capture');
  if (row.status === 'complete') requireThat(row.process_started === true && row.child_closed === true
    && row.process_status === 'complete' && row.reserved_provider_attempts === 1, 'pilot_incomplete_process');
}

async function captureSnapshot(root, manifest, plan, rows) {
  const records = [], descriptors = [], digests = [];
  for (const [index, row] of rows.entries()) {
    const descriptor = row.capture;
    if (descriptor === null) {
      requireThat(row.status !== 'complete', 'pilot_complete_without_capture');
      records.push(null); descriptors.push(null); digests.push(null);
      continue;
    }
    requireThat(row.reserved_provider_attempts === 1, 'pilot_unreserved_capture');
    await privateDirectory(join(root, slotName(row.slot)));
    const document = await readOptionalJson(join(root, descriptor.path));
    requireThat(document !== null && document.sha256 === descriptor.sha256, 'pilot_capture_digest');
    bindCapture(document.value, manifest, plan.scheduled[index], row);
    records.push(document.value); descriptors.push(descriptor); digests.push(document.sha256);
  }
  return { records, descriptors, digests };
}

async function groupAudit(manifest, descriptors, records, indices) {
  const pairs = [], loaded = new Map();
  for (const pairIndex of indices) {
    const pair = { pair_id: manifest.pairs[pairIndex].pair_id, baseline: null, tactical: null };
    for (const index of [pairIndex * 2, pairIndex * 2 + 1]) {
      const record = records[index], descriptor = descriptors[index];
      if (record === null) continue;
      pair[record.profile === 'baseline' ? 'baseline' : 'tactical'] = descriptor;
      loaded.set(descriptor.path, record);
    }
    pairs.push(pair);
  }
  return auditCaptures({ schema: CAPTURE_PAIRS_SCHEMA, model: manifest.model,
    bridge_digest: manifest.bridge.sha256, pairs }, descriptor => loaded.get(descriptor.path));
}

export async function reportPilot(path) {
  const { manifest, hash } = await pilotManifest(path), root = manifest.output_directory;
  const { plan, rows } = await readRun(root);
  requireThat(canonical(plan) === canonical(expectedPilotPlan(manifest, hash)), 'pilot_frozen_plan_mismatch');
  const snapshot = await captureSnapshot(root, manifest, plan, rows);
  const indices = manifest.pairs.map((_, index) => index);
  const audit = await groupAudit(manifest, snapshot.descriptors, snapshot.records, indices);
  const final = await readOptionalJson(join(root, 'run.result.json'));
  const expectedSummary = { ...summarize(plan, rows), audit_incomplete: audit.incomplete };
  expectedSummary.incomplete ||= audit.incomplete;
  if (final !== null) requireThat(canonical(final.value) === canonical(expectedSummary), 'pilot_final_summary_mismatch');
  const splits = {};
  for (const split of ['calibration', 'held_out']) {
    const subset = indices.filter(index => manifest.pairs[index].split === split);
    if (subset.length === 0) { splits[split] = null; continue; }
    const slots = subset.flatMap(index => [2 * index, 2 * index + 1]);
    splits[split] = { cohort: cohortDescription(subset.map(index => manifest.pairs[index])),
      ...diagnosticGroup({ ...plan, scheduled: slots.map(index => plan.scheduled[index]) },
        slots.map(index => rows[index]), slots.map(index => snapshot.records[index]),
        await groupAudit(manifest, snapshot.descriptors, snapshot.records, subset)) };
  }
  return { schema: 'ascension.jev-pilot-report.v1', profile: PILOT_PROFILE,
    evidence_kind: manifest.evidence_kind, evidence: 'producer_assertions_not_native_witnesses',
    manifest_sha256: hash, source_revision: manifest.source_revision,
    evidence_snapshot_sha256: sha256(canonical({ plan, rows, captures: snapshot.digests,
      final_summary_sha256: final?.sha256 ?? null })),
    cohort: cohortDescription(manifest.pairs),
    overall: diagnosticGroup(plan, rows, snapshot.records, audit), splits,
    finalized_record_present: final !== null,
    incomplete: final === null || expectedSummary.incomplete,
    execution_performed: false, source_revision_is_operator_assertion: true,
    missing_journals_are_not_liveness_proof: true, input_fingerprints_are_producer_assertions: true,
    hard_token_or_money_budget_enforced: false, output_tokens_and_dollar_cost_available: false,
    provider_execution_authenticated: false,
    policy_changed: false, native_outcomes_verified: false, gameplay_improvement_established: false };
}
