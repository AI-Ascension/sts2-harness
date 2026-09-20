// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';

export const BASE_REVISION = '1a075bcd43380dbe45a4bb33c2f822d6777c5a17';
export const COHORT_SCHEMA = 'ascension.jev-evaluation-cohort.v1';
export const RESULT_SCHEMA = 'ascension.jev-evaluation-result.v1';
export const PAIRS_SCHEMA = 'ascension.jev-decision-pairs.v1';
export const BRIDGE_SCHEMA = 'ascension.system-one-bridge-record.v1';
export const PROFILE = 'jev-tactical-v1';
export const ARMS = ['baseline', 'tactical'];
export const SPLITS = ['calibration', 'held_out'];
export const OUTCOMES = [
  'victory', 'defeat', 'timeout', 'infrastructure_failure',
  'refusal_stall', 'interrupted', 'not_started',
];
export const METRICS = [
  'provider_calls', 'input_tokens', 'latency_ms', 'cost_micro_usd', 'combat_hp_lost',
];
export const MAX_PAIRS = 4096;
export const MAX_JSON_BYTES = 1024 * 1024;
export const MAX_RESULTS_BYTES = 16 * 1024 * 1024;

export class ValidationError extends Error {
  constructor(code) {
    super(code);
    this.name = 'ValidationError';
    this.code = code;
  }
}


export function requireThat(condition, code) {
  if (!condition) throw new ValidationError(code);
}

export function object(value, code = 'expected_object') {
  requireThat(value !== null && typeof value === 'object' && !Array.isArray(value), code);
  return value;
}

export function exactKeys(value, required, optional = []) {
  object(value);
  const allowed = new Set([...required, ...optional]);
  requireThat(required.every(key => Object.hasOwn(value, key)), 'missing_field');
  requireThat(Object.keys(value).every(key => allowed.has(key)), 'unknown_field');
}

export function token(value) {
  requireThat(typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,159}$/.test(value),
    'invalid_identifier');
  return value;
}

export function digest(value, length = 64) {
  requireThat(typeof value === 'string' && new RegExp(`^[0-9a-f]{${length}}$`).test(value),
    'invalid_digest');
  return value;
}

export function modelPin(value) {
  token(value);
  requireThat(!/(?:^|[-_.:])(latest|preview|stable|default)(?:$|[-_.:])/i.test(value),
    'floating_model');
  return value;
}

export function nonnegative(value) {
  requireThat(typeof value === 'number' && Number.isFinite(value)
    && value >= 0 && value <= Number.MAX_SAFE_INTEGER, 'invalid_number');
  return value;
}

export function integer(value) {
  nonnegative(value);
  requireThat(Number.isSafeInteger(value), 'invalid_integer');
  return value;
}

export function unit(value) {
  nonnegative(value);
  requireThat(value <= 1, 'invalid_unit_value');
  return value;
}

export const sha256 = value => createHash('sha256').update(value).digest('hex');

// This is an evaluator-local equality encoding, NOT serde_json/JCS and NOT a verifier
// of the Rust bridge's request_digest. Exact input-file bytes are hashed separately.
export function canonical(value, depth = 0) {
  requireThat(depth <= 64, 'json_nesting_bound');
  if (value === null || typeof value === 'string' || typeof value === 'boolean') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    requireThat(Number.isFinite(value)
      && (!Number.isInteger(value) || Number.isSafeInteger(value)), 'unsafe_json_number');
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(item => canonical(item, depth + 1)).join(',')}]`;
  }
  object(value);
  return `{${Object.keys(value).sort().map(key =>
    `${JSON.stringify(key)}:${canonical(value[key], depth + 1)}`).join(',')}}`;
}

export function decodeUtf8(bytes) {
  try {
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    throw new ValidationError('invalid_utf8');
  }
}

// JSON.parse normally silently accepts duplicate keys. This syntax walk runs after
// syntax validation, and rejects duplicates even when key spellings use escapes.
function uniqueJsonKeys(text) {
  let at = 0;
  const whitespace = () => { while (/\s/.test(text[at] ?? '') && at < text.length) at++; };
  function string() {
    const start = at++;
    while (text[at] !== '"') {
      if (text[at] === '\\') at++;
      at++;
    }
    at++;
    return JSON.parse(text.slice(start, at));
  }
  function value(depth) {
    requireThat(depth <= 64, 'json_nesting_bound');
    whitespace();
    if (text[at] === '"') { string(); return; }
    if (text[at] === '{') {
      at++;
      whitespace();
      const seen = new Set();
      while (text[at] !== '}') {
        whitespace();
        const key = string();
        requireThat(!seen.has(key), 'duplicate_json_key');
        seen.add(key);
        whitespace();
        at++;
        value(depth + 1);
        whitespace();
        if (text[at] !== ',') break;
        at++;
      }
      at++;
      return;
    }
    if (text[at] === '[') {
      at++;
      whitespace();
      while (text[at] !== ']') {
        value(depth + 1);
        whitespace();
        if (text[at] !== ',') break;
        at++;
      }
      at++;
      return;
    }
    while (at < text.length && !/[\s,}\]]/.test(text[at])) at++;
  }
  value(0);
}

export function parseJson(bytes) {
  const text = decodeUtf8(bytes);
  requireThat(!text.startsWith('\ufeff'), 'byte_order_mark');
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new ValidationError('invalid_json');
  }
  uniqueJsonKeys(text);
  canonical(parsed);
  return parsed;
}

export function evidenceKind(value) {
  requireThat(value === 'synthetic' || value === 'operator_recorded', 'invalid_evidence_kind');
}

export function validatePins(pins) {
  const hashes = [
    'bridge_sha256', 'host_mod_sha256', 'protocol_sha256', 'observation_policy_sha256',
    'budget_sha256', 'baseline_policy_sha256', 'tactical_policy_sha256',
  ];
  exactKeys(pins, ['source_revision', 'model', 'game_build_id', ...hashes]);
  digest(pins.source_revision, 40);
  modelPin(pins.model);
  token(pins.game_build_id);
  for (const key of hashes) digest(pins[key]);
  requireThat(pins.baseline_policy_sha256 !== pins.tactical_policy_sha256,
    'identical_policy_pins');
}

export function validateCohort(cohort) {
  exactKeys(cohort, ['schema', 'experiment_id', 'evidence_kind', 'pins', 'pairs']);
  requireThat(cohort.schema === COHORT_SCHEMA, 'cohort_schema');
  token(cohort.experiment_id);
  evidenceKind(cohort.evidence_kind);
  validatePins(cohort.pins);
  requireThat(Array.isArray(cohort.pairs) && cohort.pairs.length > 0
    && cohort.pairs.length <= MAX_PAIRS, 'pair_count_bound');
  const ids = new Set();
  const repeats = new Set();
  const seedSplits = new Map();
  for (const pair of cohort.pairs) {
    exactKeys(pair, ['pair_id', 'seed_sha256', 'repetition', 'split']);
    token(pair.pair_id);
    digest(pair.seed_sha256);
    integer(pair.repetition);
    requireThat(pair.repetition < 100, 'repetition_bound');
    requireThat(SPLITS.includes(pair.split), 'invalid_split');
    requireThat(!ids.has(pair.pair_id), 'duplicate_pair');
    const key = `${pair.seed_sha256}:${pair.repetition}`;
    requireThat(!repeats.has(key), 'duplicate_seed_repetition');
    requireThat(!seedSplits.has(pair.seed_sha256)
      || seedSplits.get(pair.seed_sha256) === pair.split, 'seed_split_leakage');
    ids.add(pair.pair_id);
    repeats.add(key);
    seedSplits.set(pair.seed_sha256, pair.split);
  }
  return cohort;
}

export function firstArm(cohort, pair) {
  const bytes = Buffer.from(sha256(canonical([
    cohort.experiment_id, pair.seed_sha256, pair.repetition,
  ])), 'hex');
  return ARMS[bytes[0] % 2];
}

export function plan(cohort, manifestDigest) {
  validateCohort(cohort);
  digest(manifestDigest);
  return {
    schema: 'ascension.jev-evaluation-plan.v1', manifest_sha256: manifestDigest,
    evidence_kind: cohort.evidence_kind, launches_performed: 0,
    scheduled: cohort.pairs.flatMap(pair => {
      const first = firstArm(cohort, pair);
      return [first, ARMS.find(arm => arm !== first)].map((arm, slot) => ({
        pair_id: pair.pair_id, split: pair.split, seed_sha256: pair.seed_sha256,
        repetition: pair.repetition, arm, slot,
      }));
    }),
  };
}
