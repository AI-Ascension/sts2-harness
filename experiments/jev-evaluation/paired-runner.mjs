// SPDX-License-Identifier: MIT

// Standalone, explicitly authorized replay of private approved inputs through the existing bridge.
// It never dispatches decisions to a game, starts a gateway, or imports provider credentials itself.
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';
import { digest, requireThat } from './contract.mjs';
import { CAPTURE_PAIRS_SCHEMA } from './capture-records.mjs';
import { auditCaptures } from './capture-audit.mjs';
import { localRecord } from './io.mjs';
import { RUN_SCHEMA, bridgeArguments, payload } from './runner-contract.mjs';
import { createDirectory, createJson, preflight, selectedEnvironment, verifyExecutable } from './runner-io.mjs';
import { collectEvidence, slotName } from './runner-evidence.mjs';
import { armResult, reservation, summarize } from './runner-journal.mjs';
import { runBridge } from './runner-process.mjs';

export async function planRun(path) {
  const prepared = await preflight(path);
  return { schema: 'ascension.jev-paired-plan.v1', manifest_sha256: prepared.hash,
    scheduled_pairs: prepared.manifest.pairs.length, scheduled_arms: prepared.entries.length,
    maximum_reserved_provider_attempts: prepared.entries.length,
    execution_performed: false, source_revision_is_operator_assertion: true };
}

function frozenPlan(prepared) {
  const { manifest: m, hash, entries } = prepared;
  return { schema: RUN_SCHEMA, manifest_sha256: hash, source_revision: m.source_revision,
    bridge_digest: m.bridge.sha256, transport_digest: m.transport.sha256,
    evidence_kind: m.evidence_kind, budgets: m.budgets, scheduled: entries };
}

async function executeArm(prepared, entry, env, deadline, signal) {
  const { manifest: m, hash, inputs } = prepared, root = m.output_directory;
  const input = inputs[entry.pair_position];
  // Detect ordinary artifact replacement between arms. This is not an adversarial TOCTOU sandbox.
  try { await verifyExecutable(m.bridge); await verifyExecutable(m.transport); } catch {
    return armResult(hash, entry, { status: 'executable_drift' });
  }
  if (signal?.aborted || performance.now() >= deadline) return armResult(hash, entry);
  await createJson(join(root, `${slotName(entry.slot)}.pending.json`), reservation(hash, entry));
  const directory = join(root, slotName(entry.slot));
  await createDirectory(directory);
  const body = payload(input, hash, entry.slot);
  let result;
  try {
    const remaining = Math.floor(deadline - performance.now());
    result = remaining <= 0 || signal?.aborted
      ? { status: 'cancelled', stdout: Buffer.alloc(0), elapsed_ms: 0,
        process_started: false, child_closed: true }
      : await runBridge(m.bridge.path, bridgeArguments(m, entry.arm, directory), body, {
        cwd: directory, env, signal, timeoutMs: Math.min(m.budgets.per_arm_timeout_ms, remaining),
      });
    const evidence = await collectEvidence(root, m, entry, input, result);
    return armResult(hash, entry, { ...evidence, process_started: result.process_started,
      child_closed: result.child_closed, process_status: result.status,
      elapsed_ms: result.elapsed_ms, reserved_provider_attempts: 1 });
  } finally { body.fill(0); result?.stdout.fill(0); }
}

async function finish(prepared, plan, rows) {
  const { manifest: m } = prepared, root = m.output_directory;
  const pairs = { schema: CAPTURE_PAIRS_SCHEMA, model: m.model, bridge_digest: m.bridge.sha256,
    pairs: m.pairs.map(pair => ({ pair_id: pair.pair_id, baseline: null, tactical: null })) };
  for (const [index, entry] of prepared.entries.entries()) {
    pairs.pairs[entry.pair_position][entry.arm] = rows[index].capture;
  }
  const pairsPath = join(root, 'pairs.json');
  await createJson(pairsPath, pairs);
  const audit = await auditCaptures(pairs, descriptor => localRecord(pairsPath, descriptor));
  await createJson(join(root, 'audit.json'), audit);
  // Keep calibration and held-out diagnostic denominators separately inspectable.
  for (const split of ['calibration', 'held_out']) {
    const subset = pairs.pairs.filter((_, index) => m.pairs[index].split === split);
    if (subset.length > 0) await createJson(join(root, `audit-${split}.json`),
      await auditCaptures({ ...pairs, pairs: subset }, descriptor => localRecord(pairsPath, descriptor)));
  }
  const summary = summarize(plan, rows);
  summary.audit_incomplete = audit.incomplete;
  summary.incomplete ||= audit.incomplete;
  await createJson(join(root, 'run.result.json'), summary);
  return summary;
}

export async function executeRun(path, approvedHash, { signal, environment = process.env } = {}) {
  digest(approvedHash);
  const prepared = await preflight(path);
  requireThat(prepared.hash === approvedHash, 'runner_approval_mismatch');
  // Values are kept only in memory and handed to the reviewed transport via a cleared environment.
  const env = selectedEnvironment(prepared.manifest, environment);
  const { manifest: m, hash, entries } = prepared, plan = frozenPlan(prepared);
  await createDirectory(m.output_directory);
  await createJson(join(m.output_directory, 'run.pending.json'), plan);
  const deadline = performance.now() + m.budgets.total_timeout_ms;
  const rows = [];
  let stopped = false;
  try {
    for (const entry of entries) {
      const row = stopped || signal?.aborted || performance.now() >= deadline
        ? armResult(hash, entry)
        : await executeArm(prepared, entry, env, deadline, signal);
      rows.push(row);
      await createJson(join(m.output_directory, `${slotName(entry.slot)}.result.json`), row);
      stopped ||= row.status === 'executable_drift' || row.child_closed !== true;
    }
    return await finish(prepared, plan, rows);
  } finally {
    prepared.inputs.length = 0;
    for (const key of Object.keys(env)) delete env[key];
  }
}
