// SPDX-License-Identifier: MIT

// Hand-authored MIT synthetic journals. These do NOT pretend to be compiled/provider executions.
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sha256 } from './contract.mjs';
import { RUNNER_SCHEMA } from './runner-contract.mjs';
import { CAPTURE_SCHEMA, CAPTURE_PAIRS_SCHEMA, modelFingerprint, validateCapture } from './capture-records.mjs';
import { auditCaptures } from './capture-audit.mjs';
import { armResult, reservation, summarize } from './runner-journal.mjs';
import { expectedPilotPlan } from './pilot-profile.mjs';
import { slotName } from './runner-evidence.mjs';

export const json = async path => JSON.parse(await readFile(path, 'utf8'));
export async function writeJson(path, value) {
  const bytes = Buffer.from(`${JSON.stringify(value)}\n`);
  await writeFile(path, bytes, { mode: 0o600 });
  return sha256(bytes);
}

export async function pilotFixture(t, modes = []) {
  const base = fileURLToPath(new URL('../../target/jev-pilot-tests/', import.meta.url));
  await mkdir(base, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(base, 'case-'));
  await chmod(root, 0o700);
  t.after(() => rm(root, { recursive: true, force: true }));
  const bin = join(root, 'Program Files'); await mkdir(bin, { mode: 0o700 });
  const executable = Buffer.from('#!/bin/false\n');
  const bridge = join(bin, 'Synthetic bridge'), transport = join(bin, 'Synthetic transport');
  await writeFile(bridge, executable, { mode: 0o700 });
  await writeFile(transport, executable, { mode: 0o700 });
  const manifest = { schema: RUNNER_SCHEMA, experiment_id: 'synthetic-private-identifier',
    evidence_kind: 'synthetic', source_revision: '907982c8adab0ec25fec55eb918901c1152ba569',
    model: 'jev-1.13.0', bridge: { path: bridge, sha256: sha256(executable) },
    transport: { path: transport, sha256: sha256(executable) },
    inherited_environment: ['TYPESAFE_API_KEY'], confidence_gate_percent: 20,
    budgets: { max_pairs: 10, max_provider_attempts: 20, per_arm_timeout_ms: 1000,
      total_timeout_ms: 20000, max_total_input_bytes: 128 * 1024 * 10 },
    output_directory: join(root, 'run'), pairs: [] };
  for (let index = 0; index < 10; index++) {
    const count = modes[index] === 'forced' ? 1 : modes[index] === 'fallback' ? 25 : 2;
    const input = { model_execution_id: `private-original-${index}`, objective: 'PRIVATE_OBJECTIVE',
      hard_constraints: [], legal_action_ids: Array.from({ length: count }, (_, i) => `private-action-${i}`),
      observation: { state_id: `private-state-${index}`, generation: 1,
        player: { hp: 30 + index, hand: [] }, state: { state: 'combat' } } };
    const inputPath = `input ${index}.json`, inputHash = await writeJson(join(root, inputPath), input);
    manifest.pairs.push({ pair_id: `private-pair-${index}`, input_path: inputPath,
      input_sha256: inputHash, cluster_sha256: sha256(`cluster-${index}`), repetition: 0,
      split: index < 5 ? 'calibration' : 'held_out' });
  }
  const path = join(root, 'manifest.json');
  const f = { root, path, manifest, modes, rows: [], records: [],
    save: async () => writeJson(path, manifest) };
  await f.save(); return f;
}

function syntheticCapture(manifest, entry, mode) {
  const tactical = entry.arm === 'tactical', pair = entry.pair_position;
  const count = mode === 'forced' ? 1 : mode === 'fallback' ? 25 : 2;
  const record = { schema: CAPTURE_SCHEMA, profile: tactical ? 'jev-tactical-v1' : 'baseline',
    bridge_digest: manifest.bridge.sha256, model_execution_id_digest: entry.execution_id_digest,
    input_digest: sha256(`synthetic-input-${pair}`), catalog_digest: sha256(`catalog-${count}`),
    catalog_count: count, requested_model_digest: modelFingerprint(manifest.model),
    confidence_gate: manifest.confidence_gate_percent / 100, status: 'complete',
    provider_attempts: mode === 'forced' ? 0 : 1, elapsed_ms: tactical ? 150 : 100 };
  if (mode === 'failed') return { ...record, status: 'failed' };
  if (mode === 'pending') return { ...record, status: 'pending', provider_attempts: null, elapsed_ms: null };
  record.provider = mode === 'forced' ? null : {
    request_digest: sha256(`request-${pair}-${entry.arm}`), shared_request_digest: sha256(`shared-${pair}`),
    question_set_digest: sha256(`questions-${entry.arm}`),
    response_model_digest: mode === 'model_drift' && tactical ? 'a'.repeat(64) : modelFingerprint(manifest.model),
    input_tokens: mode === 'missing_usage' && tactical ? null : tactical ? 200 : 100 };
  record.decision = { kind: 'action', selected_index: tactical && count === 2 ? 1 : 0, candidate_index: null };
  record.tactical = null;
  if (tactical && ['forced', 'fallback'].includes(mode)) {
    record.tactical = { applied: false, fallback_reason: mode === 'forced'
      ? 'forced_action' : 'candidate_bound_or_incomplete_catalog' };
  } else if (tactical) {
    const row = (index, score) => ({ index, scores: Array(6).fill(score), evidence: 0.95,
      min_confidence: 0.9, utility: score });
    const rows = [row(1, 0.9), row(0, 0.2)];
    if (mode === 'low_evidence') rows[1].evidence = 0.1;
    if (mode === 'low_confidence') rows[0].min_confidence = 0.1;
    if (mode === 'low_margin') rows[1] = row(0, 0.85);
    if (mode === 'low_safety') {
      rows[0].scores[5] = 0.4;
      rows[0].utility = rows[0].scores.reduce((sum, score, i) => sum + score * [3, 3, 2, 2, 2, 4][i], 0) / 16;
    }
    if (mode.startsWith('low_')) record.decision = { kind: 'reobserve', selected_index: null, candidate_index: null };
    record.tactical = { applied: true, rows, within_request_index: 0,
      minimum_evidence: 0.8, minimum_margin: 0.1, minimum_safety: 0.5 };
  }
  return validateCapture(record);
}

export async function syntheticRun(f, { finalize = true } = {}) {
  const hash = sha256(await readFile(f.path)), plan = expectedPilotPlan(f.manifest, hash);
  const root = f.manifest.output_directory; await mkdir(root, { mode: 0o700 });
  await writeJson(join(root, 'run.pending.json'), plan);
  for (const entry of plan.scheduled) {
    const mode = f.modes[entry.pair_position] ?? 'success';
    if (mode === 'not_started') {
      f.rows.push(armResult(hash, entry)); f.records.push(null); continue;
    }
    await writeJson(join(root, `${slotName(entry.slot)}.pending.json`), reservation(hash, entry));
    if (mode === 'interrupted_unknown') {
      f.rows.push(armResult(hash, entry, { status: 'interrupted_unknown', process_started: null,
        child_closed: null, reserved_provider_attempts: 1, observed_provider_attempts: null, input_tokens: null }));
      f.records.push(null); continue;
    }
    const record = syntheticCapture(f.manifest, entry, mode);
    await mkdir(join(root, slotName(entry.slot)), { mode: 0o700 });
    const kind = record.status === 'pending' ? 'pending' : 'result';
    const capturePath = `${slotName(entry.slot)}/attempt-0000.${kind}.json`;
    const captureHash = await writeJson(join(root, capturePath), record);
    const status = record.status === 'complete' ? 'complete' : 'bridge_failed';
    const row = armResult(hash, entry, { status, process_started: true, child_closed: true,
      process_status: status, elapsed_ms: entry.arm === 'tactical' ? 700 : 500,
      reserved_provider_attempts: 1, observed_provider_attempts: record.provider_attempts,
      input_tokens: record.status === 'complete' ? record.provider?.input_tokens ?? (record.provider === null ? 0 : null) : null,
      capture_elapsed_ms: record.elapsed_ms, capture: { path: capturePath, sha256: captureHash } });
    await writeJson(join(root, `${slotName(entry.slot)}.result.json`), row);
    f.rows.push(row); f.records.push(record);
  }
  const pairs = f.manifest.pairs.map(pair => ({ pair_id: pair.pair_id, baseline: null, tactical: null }));
  const byPath = new Map();
  for (const entry of plan.scheduled) {
    pairs[entry.pair_position][entry.arm] = f.rows[entry.slot].capture;
    if (f.records[entry.slot]) byPath.set(f.rows[entry.slot].capture.path, f.records[entry.slot]);
  }
  const audit = await auditCaptures({ schema: CAPTURE_PAIRS_SCHEMA, model: f.manifest.model,
    bridge_digest: f.manifest.bridge.sha256, pairs }, descriptor => byPath.get(descriptor.path));
  const summary = { ...summarize(plan, f.rows), audit_incomplete: audit.incomplete };
  summary.incomplete ||= audit.incomplete;
  if (finalize) await writeJson(join(root, 'run.result.json'), summary);
  return { plan, audit, summary };
}

export async function rewriteCapture(f, slot, change) {
  const row = f.rows[slot], record = f.records[slot]; change(record);
  row.capture.sha256 = await writeJson(join(f.manifest.output_directory, row.capture.path), record);
  await writeJson(join(f.manifest.output_directory, `${slotName(slot)}.result.json`), row);
}
