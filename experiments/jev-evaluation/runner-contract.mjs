// SPDX-License-Identifier: MIT

// A private experiment plan, not runtime admission or a native-game benchmark manifest.
import { isAbsolute, resolve } from 'node:path';
import { canonical, digest, evidenceKind, exactKeys, integer, modelPin, object,
  requireThat, sha256, token } from './contract.mjs';

export const RUNNER_SCHEMA = 'ascension.jev-paired-runner.v1';
export const RUN_SCHEMA = 'ascension.jev-paired-execution.v1';
export const ARM_SCHEMA = 'ascension.jev-paired-arm.v1';
export const MAX_INPUT = 128 * 1024;
export const MAX_PAIRS = 256;
export const ARMS = ['baseline', 'tactical'];
export const TERMINALS = ['complete', 'bridge_failed', 'timeout', 'cancelled',
  'spawn_failed', 'output_bound', 'io_failed', 'capture_invalid', 'capture_missing',
  'stdout_mismatch', 'executable_drift', 'not_started', 'interrupted_unknown'];

export function absolutePath(value) {
  requireThat(typeof value === 'string' && Buffer.byteLength(value) <= 4096
    && !/\p{Cc}/u.test(value) && isAbsolute(value) && resolve(value) === value,
  'runner_absolute_path');
  return value;
}

export function inputPath(value) {
  requireThat(typeof value === 'string' && value.length > 0 && Buffer.byteLength(value) <= 512
    && !isAbsolute(value) && !/[\\\p{Cc}]/u.test(value)
    && value.split('/').every(part => part !== '' && part !== '.' && part !== '..'),
  'runner_input_path');
  return value;
}

function executable(value) {
  exactKeys(value, ['path', 'sha256']);
  absolutePath(value.path); digest(value.sha256);
}

export function environmentNames(names) {
  requireThat(Array.isArray(names) && names.length <= 16, 'runner_environment');
  const denied = /^(?:LD_|DYLD_|PYTHON|NODE_|BASH_ENV$|ENV$|SHELLOPTS$|BASHOPTS$|JEV_CONTEXT_LOG$)/;
  const seen = new Set();
  for (const name of names) {
    requireThat(typeof name === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(name)
      && !denied.test(name) && !seen.has(name), 'runner_environment');
    seen.add(name);
  }
}

export function validateRunner(manifest) {
  exactKeys(manifest, ['schema', 'experiment_id', 'evidence_kind', 'source_revision', 'model',
    'bridge', 'transport', 'inherited_environment', 'confidence_gate_percent', 'budgets',
    'output_directory', 'pairs']);
  requireThat(manifest.schema === RUNNER_SCHEMA, 'runner_schema');
  token(manifest.experiment_id); evidenceKind(manifest.evidence_kind);
  digest(manifest.source_revision, 40); modelPin(manifest.model);
  executable(manifest.bridge); executable(manifest.transport);
  environmentNames(manifest.inherited_environment); absolutePath(manifest.output_directory);
  requireThat(integer(manifest.confidence_gate_percent) <= 100, 'runner_gate');
  exactKeys(manifest.budgets, ['max_pairs', 'max_provider_attempts', 'per_arm_timeout_ms',
    'total_timeout_ms', 'max_total_input_bytes']);
  const b = manifest.budgets;
  requireThat(integer(b.max_pairs) >= 1 && b.max_pairs <= MAX_PAIRS, 'runner_pair_budget');
  requireThat(integer(b.max_provider_attempts) >= 2 && b.max_provider_attempts <= 2 * MAX_PAIRS,
    'runner_attempt_budget');
  requireThat(integer(b.per_arm_timeout_ms) >= 25 && b.per_arm_timeout_ms <= 120000,
    'runner_arm_deadline');
  requireThat(integer(b.total_timeout_ms) >= 25 && b.total_timeout_ms <= 3600000,
    'runner_total_deadline');
  requireThat(integer(b.max_total_input_bytes) >= 1 && b.max_total_input_bytes <= MAX_PAIRS * MAX_INPUT,
    'runner_input_budget');
  requireThat(Array.isArray(manifest.pairs) && manifest.pairs.length >= 1
    && manifest.pairs.length <= b.max_pairs && 2 * manifest.pairs.length <= b.max_provider_attempts,
  'runner_unfunded_plan');
  const ids = new Set(), repeats = new Set(), clusters = new Map(), inputs = new Map();
  for (const pair of manifest.pairs) {
    exactKeys(pair, ['pair_id', 'input_path', 'input_sha256', 'cluster_sha256', 'repetition', 'split']);
    token(pair.pair_id); inputPath(pair.input_path); digest(pair.input_sha256); digest(pair.cluster_sha256);
    requireThat(integer(pair.repetition) < 100, 'runner_repetition');
    requireThat(['calibration', 'held_out'].includes(pair.split), 'runner_split');
    const key = `${pair.input_sha256}:${pair.repetition}`;
    requireThat(!ids.has(pair.pair_id) && !repeats.has(key), 'runner_duplicate_pair');
    for (const [map, identity] of [[clusters, pair.cluster_sha256], [inputs, pair.input_sha256]]) {
      requireThat(!map.has(identity) || map.get(identity) === pair.split, 'runner_split_leakage');
      map.set(identity, pair.split);
    }
    ids.add(pair.pair_id); repeats.add(key);
  }
  return manifest;
}

export function validateInput(input) {
  object(input); object(input.observation);
  requireThat(typeof input.model_execution_id === 'string' && input.model_execution_id.length > 0,
    'runner_input_execution');
  const ids = input.legal_action_ids;
  requireThat(Array.isArray(ids) && ids.length > 0 && ids.length <= 256, 'runner_input_catalog');
  const seen = new Set();
  for (const id of ids) {
    requireThat(typeof id === 'string' && id.length > 0 && Buffer.byteLength(id) <= 512
      && !/\p{Cc}/u.test(id) && !seen.has(id), 'runner_input_catalog');
    seen.add(id);
  }
  return input;
}

// Restrict generated IDs to ASCII; this string-only fingerprint also matches serde_json.
export const executionId = (manifestHash, slot) => `jev-pair:${manifestHash}:${slot}`;
export const executionFingerprint = id => sha256(Buffer.from(
  `ascension.jev-capture.v1/model_execution_id\0${JSON.stringify(id)}`));

export function schedule(manifest, manifestHash) {
  validateRunner(manifest); digest(manifestHash);
  return manifest.pairs.flatMap((pair, pairIndex) => {
    const first = parseInt(sha256(canonical([manifest.experiment_id, pair.input_sha256,
      pair.repetition])).slice(0, 2), 16) % 2;
    return [ARMS[first], ARMS[1 - first]].map((arm, order) => {
      const slot = pairIndex * 2 + order;
      return { slot, pair_position: pairIndex, arm, split: pair.split,
        execution_id_digest: executionFingerprint(executionId(manifestHash, slot)) };
    });
  });
}

export function payload(input, manifestHash, slot) {
  const bytes = Buffer.from(JSON.stringify({ ...input, model_execution_id: executionId(manifestHash, slot) }));
  requireThat(bytes.length <= MAX_INPUT, 'runner_payload_bound');
  return bytes;
}

export function bridgeArguments(manifest, arm, directory) {
  const args = ['--model', manifest.model, '--transport', manifest.transport.path,
    '--gate', String(manifest.confidence_gate_percent)];
  if (arm === 'tactical') args.push('--tactical');
  args.push('--audit-dir', directory);
  return args;
}
