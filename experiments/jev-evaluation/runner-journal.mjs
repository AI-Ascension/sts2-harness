// SPDX-License-Identifier: MIT

import { join, resolve } from 'node:path';
import { canonical, digest, exactKeys, integer, requireThat } from './contract.mjs';
import { ARM_SCHEMA, RUN_SCHEMA, TERMINALS } from './runner-contract.mjs';
import { privateDirectory, readOptionalJson } from './runner-io.mjs';
import { slotName } from './runner-evidence.mjs';

export function reservation(hash, entry) {
  return { schema: 'ascension.jev-paired-reservation.v1', manifest_sha256: hash,
    slot: entry.slot, execution_id_digest: entry.execution_id_digest, reserved_provider_attempts: 1 };
}

export function armResult(hash, entry, fields = {}) {
  return { schema: ARM_SCHEMA, manifest_sha256: hash, slot: entry.slot,
    execution_id_digest: entry.execution_id_digest, status: 'not_started', process_started: false,
    child_closed: true, process_status: null, elapsed_ms: null, reserved_provider_attempts: 0,
    observed_provider_attempts: 0, input_tokens: 0, capture_elapsed_ms: null, capture: null, ...fields };
}

function validateArm(row, hash, entry) {
  exactKeys(row, Object.keys(armResult(hash, entry)));
  requireThat(row.schema === ARM_SCHEMA && row.manifest_sha256 === hash && row.slot === entry.slot
    && row.execution_id_digest === entry.execution_id_digest && TERMINALS.includes(row.status), 'runner_journal_identity');
  requireThat([true, false, null].includes(row.process_started)
    && [true, false, null].includes(row.child_closed), 'runner_journal_state');
  requireThat(row.process_status === null || ['complete', 'bridge_failed', 'timeout', 'cancelled',
    'spawn_failed', 'output_bound', 'io_failed'].includes(row.process_status), 'runner_journal_state');
  for (const key of ['elapsed_ms', 'observed_provider_attempts', 'input_tokens', 'capture_elapsed_ms']) {
    if (row[key] !== null) integer(row[key]);
  }
  requireThat(row.observed_provider_attempts === null || row.observed_provider_attempts <= 1, 'runner_journal_attempts');
  requireThat(integer(row.reserved_provider_attempts) <= 1, 'runner_journal_attempts');
  if (row.capture !== null) {
    exactKeys(row.capture, ['path', 'sha256']); digest(row.capture.sha256);
    requireThat(['pending', 'result'].some(kind => row.capture.path ===
      `${slotName(entry.slot)}/attempt-0000.${kind}.json`), 'runner_journal_capture');
  }
  return row;
}

export function summarize(plan, rows, phase = 'finished') {
  const counts = Object.fromEntries(TERMINALS.map(status => [status, 0]));
  for (const row of rows) counts[row.status] += 1;
  return { schema: 'ascension.jev-paired-summary.v1', phase,
    evidence_kind: plan.evidence_kind, scheduled_arms: plan.scheduled.length, counts,
    reserved_provider_attempts: rows.reduce((sum, row) => sum + row.reserved_provider_attempts, 0),
    observed_provider_attempts_known_sum: rows.reduce((sum, row) => sum + (row.observed_provider_attempts ?? 0), 0),
    observed_provider_attempts_unknown_arms: rows.filter(row => row.observed_provider_attempts === null).length,
    process_status_counts: rows.reduce((counts, row) => {
      if (row.process_status !== null) counts[row.process_status] = (counts[row.process_status] ?? 0) + 1;
      return counts;
    }, {}),
    by_arm: Object.fromEntries(['baseline', 'tactical'].map(arm => {
      const subset = rows.filter((_, index) => plan.scheduled[index].arm === arm);
      const measurements = {};
      for (const key of ['observed_provider_attempts', 'input_tokens', 'elapsed_ms', 'capture_elapsed_ms']) {
        measurements[`${key}_known_sum`] = subset.reduce((sum, row) => integer(sum + (row[key] ?? 0)), 0);
        measurements[`${key}_unknown_arms`] = subset.filter(row => row[key] === null).length;
      }
      return [arm, { scheduled: subset.length, complete: subset.filter(row => row.status === 'complete').length,
        ...measurements }];
    })),
    process_elapsed_ms_known_sum: rows.reduce((sum, row) => sum + (row.elapsed_ms ?? 0), 0),
    incomplete: counts.complete !== plan.scheduled.length,
    gameplay_improvement_established: false };
}

// Read-only accounting, never resumption. An outstanding reservation may still be active.
export async function inspectRun(directory) {
  const root = resolve(directory); await privateDirectory(root);
  const document = await readOptionalJson(join(root, 'run.pending.json'), 1024 * 1024);
  requireThat(document !== null, 'runner_plan_missing');
  const plan = document.value;
  exactKeys(plan, ['schema', 'manifest_sha256', 'source_revision', 'bridge_digest', 'transport_digest',
    'evidence_kind', 'budgets', 'scheduled']);
  requireThat(plan.schema === RUN_SCHEMA && ['synthetic', 'operator_recorded'].includes(plan.evidence_kind),
    'runner_plan_schema');
  for (const key of ['manifest_sha256', 'bridge_digest', 'transport_digest']) digest(plan[key]);
  digest(plan.source_revision, 40);
  requireThat(Array.isArray(plan.scheduled) && plan.scheduled.length > 0
    && plan.scheduled.length <= 512 && plan.scheduled.length % 2 === 0, 'runner_plan_slots');
  const rows = [], executions = new Set();
  for (const [slot, entry] of plan.scheduled.entries()) {
    exactKeys(entry, ['slot', 'pair_position', 'arm', 'split', 'execution_id_digest']);
    digest(entry.execution_id_digest);
    requireThat(entry.slot === slot && entry.pair_position === Math.floor(slot / 2)
      && ['baseline', 'tactical'].includes(entry.arm) && ['calibration', 'held_out'].includes(entry.split)
      && !executions.has(entry.execution_id_digest), 'runner_plan_slots');
    if (slot % 2) requireThat(entry.arm !== plan.scheduled[slot - 1].arm
      && entry.split === plan.scheduled[slot - 1].split, 'runner_plan_slots');
    executions.add(entry.execution_id_digest);
    const pending = await readOptionalJson(join(root, `${slotName(slot)}.pending.json`));
    const result = await readOptionalJson(join(root, `${slotName(slot)}.result.json`));
    if (pending !== null) requireThat(canonical(pending.value) ===
      canonical(reservation(plan.manifest_sha256, entry)), 'runner_reservation');
    if (result !== null) {
      const row = validateArm(result.value, plan.manifest_sha256, entry);
      requireThat((pending !== null) === (row.reserved_provider_attempts === 1), 'runner_reservation');
      rows.push(row);
    } else rows.push(armResult(plan.manifest_sha256, entry, pending === null ? {} : {
      status: 'interrupted_unknown', process_started: null, child_closed: null,
      reserved_provider_attempts: 1, observed_provider_attempts: null, input_tokens: null }));
  }
  return { ...summarize(plan, rows, 'inspection_not_liveness_proof'), pair_audit_performed: false };
}
