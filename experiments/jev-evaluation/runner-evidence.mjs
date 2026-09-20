// SPDX-License-Identifier: MIT

import { opendir } from 'node:fs/promises';
import { join } from 'node:path';
import { canonical, exactKeys, integer, parseJson, requireThat } from './contract.mjs';
import { modelFingerprint, validateCapture } from './capture-records.mjs';
import { readOptionalJson } from './runner-io.mjs';

export const slotName = slot => `slot-${String(slot).padStart(4, '0')}`;
const HEADER = ['schema', 'profile', 'bridge_digest', 'model_execution_id_digest',
  'input_digest', 'catalog_digest', 'catalog_count', 'requested_model_digest', 'confidence_gate'];

export const sortedCatalog = ids => [...ids].sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));

function boundIdentity(record, manifest, entry, input) {
  validateCapture(record);
  requireThat(record.profile === (entry.arm === 'tactical' ? 'jev-tactical-v1' : 'baseline')
    && record.bridge_digest === manifest.bridge.sha256
    && record.model_execution_id_digest === entry.execution_id_digest
    && record.requested_model_digest === modelFingerprint(manifest.model)
    && record.confidence_gate === manifest.confidence_gate_percent / 100
    && record.catalog_count === input.legal_action_ids.length, 'runner_capture_identity');
}

function stdoutDecision(bytes, record, input) {
  const value = parseJson(bytes), action = value.decision === 'action';
  requireThat(action || value.decision === 'reobserve', 'runner_stdout');
  exactKeys(value, action ? ['decision', 'action_id', 'rationale', 'confidence'] : ['decision', 'rationale'],
    action ? [] : ['candidate_action_id', 'candidate_confidence']);
  requireThat(typeof value.rationale === 'string' && value.rationale.length > 0
    && value.rationale.length <= 512 && /^[\x20-\x7e]+$/.test(value.rationale), 'runner_stdout');
  const ids = sortedCatalog(input.legal_action_ids);
  requireThat(value.decision === record.decision.kind, 'runner_stdout');
  if (action) {
    requireThat(integer(value.confidence) <= 100
      && ids[record.decision.selected_index] === value.action_id, 'runner_stdout');
  } else {
    const candidate = Object.hasOwn(value, 'candidate_action_id');
    requireThat(candidate === Object.hasOwn(value, 'candidate_confidence')
      && candidate === (record.decision.candidate_index !== null), 'runner_stdout');
    if (candidate) requireThat(integer(value.candidate_confidence) <= 100
      && ids[record.decision.candidate_index] === value.candidate_action_id, 'runner_stdout');
  }
}

// Sidecars are still producer assertions. Binding stdout here does not prove host settlement.
export async function collectEvidence(root, manifest, entry, input, processResult) {
  const directory = slotName(entry.slot), path = join(root, directory);
  const unknown = { input_tokens: processResult.process_started ? null : 0, capture_elapsed_ms: null };
  try {
    const names = [];
    for await (const item of await opendir(path)) {
      names.push(item.name);
      requireThat(names.length <= 2, 'runner_capture_files');
    }
    if (names.length === 0) return { status: processResult.status === 'complete' ? 'capture_missing'
      : processResult.status, capture: null, observed_provider_attempts: processResult.process_started ? null : 0, ...unknown };
    requireThat(names.length <= 2 && names.every(name =>
      ['attempt-0000.pending.json', 'attempt-0000.result.json'].includes(name)), 'runner_capture_files');
    const pending = await readOptionalJson(join(path, 'attempt-0000.pending.json'));
    requireThat(pending !== null && pending.value.status === 'pending', 'runner_capture_pending');
    boundIdentity(pending.value, manifest, entry, input);
    const result = await readOptionalJson(join(path, 'attempt-0000.result.json'));
    const item = result ?? pending;
    if (result !== null) {
      requireThat(result.value.status !== 'pending', 'runner_capture_terminal');
      boundIdentity(result.value, manifest, entry, input);
      requireThat(HEADER.every(key => canonical(result.value[key]) === canonical(pending.value[key])),
        'runner_capture_identity');
    }
    const observed = item.value.provider_attempts;
    const measurements = { input_tokens: item.value.status === 'complete'
      ? (observed === 0 ? 0 : item.value.provider.input_tokens) : null,
      capture_elapsed_ms: item.value.elapsed_ms };
    const capture = { path: `${directory}/attempt-0000.${result === null ? 'pending' : 'result'}.json`,
      sha256: item.sha256 };
    if (processResult.status === 'complete') {
      requireThat(result?.value.status === 'complete', 'runner_capture_incomplete');
      try { stdoutDecision(processResult.stdout, result.value, input); } catch {
        return { status: 'stdout_mismatch', capture: null, observed_provider_attempts: observed, ...measurements };
      }
      return { status: 'complete', capture, observed_provider_attempts: observed, ...measurements };
    }
    // A complete sidecar after a failed process is quarantined, never admitted as a successful arm.
    return { status: processResult.status, capture: item.value.status === 'complete' ? null : capture,
      observed_provider_attempts: observed, ...measurements };
  } catch {
    return { status: 'capture_invalid', capture: null,
      observed_provider_attempts: processResult.process_started ? null : 0, ...unknown };
  }
}
