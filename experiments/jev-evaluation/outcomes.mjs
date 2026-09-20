// SPDX-License-Identifier: MIT

import {
  ARMS, SPLITS, OUTCOMES, METRICS, RESULT_SCHEMA, canonical, digest, exactKeys,
  firstArm, integer, nonnegative, requireThat, token, validateCohort,
} from './contract.mjs';

const terminal = row => row && ['victory', 'defeat'].includes(row.outcome);
const sum = values => values.reduce((left, right) => left + right, 0);
const mean = values => values.length ? sum(values) / values.length : null;

function validateResult(row, cohort, manifestDigest, pairs, identities) {
  exactKeys(row, [
    'schema', 'manifest_sha256', 'pair_id', 'arm', 'slot', 'pins', 'evidence_kind',
    'run_id', 'episode_id', 'started', 'outcome', 'terminal_witness_sha256', ...METRICS,
  ]);
  requireThat(row.schema === RESULT_SCHEMA, 'result_schema');
  requireThat(row.manifest_sha256 === manifestDigest, 'manifest_digest_mismatch');
  requireThat(canonical(row.pins) === canonical(cohort.pins), 'result_pin_mismatch');
  requireThat(row.evidence_kind === cohort.evidence_kind, 'mixed_evidence_kind');
  requireThat(pairs.has(row.pair_id), 'unplanned_pair');
  requireThat(ARMS.includes(row.arm), 'invalid_arm');
  const expectedSlot = row.arm === firstArm(cohort, pairs.get(row.pair_id)) ? 0 : 1;
  requireThat(row.slot === expectedSlot, 'execution_order_mismatch');
  requireThat(OUTCOMES.includes(row.outcome), 'invalid_outcome');
  requireThat(typeof row.started === 'boolean'
    && row.started === (row.outcome !== 'not_started'), 'outcome_start_mismatch');
  for (const namespace of ['run_id', 'episode_id']) {
    token(row[namespace]);
    const key = `${namespace}:${row[namespace]}`;
    requireThat(!identities.has(key), 'duplicate_execution_identity');
    identities.add(key);
  }
  if (terminal(row)) digest(row.terminal_witness_sha256);
  else requireThat(row.terminal_witness_sha256 === null, 'nonterminal_witness');
  for (const metric of METRICS) {
    if (row[metric] !== null) nonnegative(row[metric]);
    if (row[metric] !== null && metric !== 'latency_ms' && metric !== 'combat_hp_lost') {
      integer(row[metric]);
    }
    requireThat(row.started || row[metric] === null || row[metric] === 0,
      'not_started_has_measurements');
  }
}


function metricSummary(rows, metric) {
  const known = rows.filter(row => row?.[metric] !== null && row?.[metric] !== undefined);
  const total = sum(known.map(row => row[metric]));
  requireThat(Number.isFinite(total) && total <= Number.MAX_SAFE_INTEGER,
    'aggregate_number_bound');
  return {
    observed_total: known.length ? total : null, known_count: known.length,
    unknown_count: rows.length - known.length,
    complete_total: known.length === rows.length && rows.length ? total : null,
  };
}

function armSummary(rows) {
  const counts = Object.fromEntries([...OUTCOMES, 'unreported'].map(value => [value, 0]));
  for (const row of rows) counts[row?.outcome ?? 'unreported'] += 1;
  const started = rows.filter(row => row?.started).length;
  return {
    scheduled: rows.length, reported: rows.filter(Boolean).length, known_started: started,
    start_status_unknown: counts.unreported, outcomes: counts,
    recorded_victory_fraction_of_scheduled: rows.length ? counts.victory / rows.length : null,
    terminal_fraction_of_known_started: started ? (counts.victory + counts.defeat) / started : null,
    metrics: Object.fromEntries(METRICS.map(metric => [metric, metricSummary(rows, metric)])),
  };
}

function pairedSummary(pairs, indexed) {
  const reported = [];
  const terminalPairs = [];
  for (const pair of pairs) {
    const baseline = indexed.get(`${pair.pair_id}:baseline`);
    const tactical = indexed.get(`${pair.pair_id}:tactical`);
    if (!baseline || !tactical) continue;
    const delta = Number(tactical.outcome === 'victory') - Number(baseline.outcome === 'victory');
    const observation = { seed: pair.seed_sha256, delta, baseline, tactical };
    reported.push(observation);
    if (terminal(baseline) && terminal(tactical)) terminalPairs.push(observation);
  }
  const seedMeans = new Map();
  for (const row of reported) {
    const values = seedMeans.get(row.seed) ?? [];
    values.push(row.delta);
    seedMeans.set(row.seed, values);
  }
  return {
    scheduled_pairs: pairs.length, both_reported_pairs: reported.length,
    unreported_pair_count: pairs.length - reported.length,
    distinct_reported_seed_clusters: seedMeans.size,
    operational_victory_difference_reported_pairs: mean(reported.map(row => row.delta)),
    operational_victory_difference_equal_seed_weight: mean([...seedMeans.values()].map(mean)),
    both_terminal_gameplay_pairs: terminalPairs.length,
    terminal_only_victory_difference: mean(terminalPairs.map(row => row.delta)),
    terminal_only_is_selection_biased: terminalPairs.length !== pairs.length,
    metric_differences: Object.fromEntries(METRICS.map(metric => {
      const known = reported.filter(row => row.baseline[metric] !== null
        && row.tactical[metric] !== null);
      return [metric, {
        paired_mean_tactical_minus_baseline: mean(known.map(row =>
          row.tactical[metric] - row.baseline[metric])),
        known_pairs: known.length, unknown_pairs: pairs.length - known.length,
      }];
    })),
  };
}

export function compare(cohort, manifestDigest, rows) {
  validateCohort(cohort);
  digest(manifestDigest);
  requireThat(Array.isArray(rows) && rows.length <= cohort.pairs.length * 2, 'result_count_bound');
  const pairs = new Map(cohort.pairs.map(pair => [pair.pair_id, pair]));
  const indexed = new Map();
  const identities = new Set();
  for (const row of rows) {
    validateResult(row, cohort, manifestDigest, pairs, identities);
    const key = `${row.pair_id}:${row.arm}`;
    requireThat(!indexed.has(key), 'duplicate_arm_result');
    indexed.set(key, row);
  }
  const splits = {};
  for (const split of SPLITS) {
    const selected = cohort.pairs.filter(pair => pair.split === split);
    splits[split] = {
      arms: Object.fromEntries(ARMS.map(arm => [arm,
        armSummary(selected.map(pair => indexed.get(`${pair.pair_id}:${arm}`)))])),
      paired: pairedSummary(selected, indexed),
    };
  }
  const missing = cohort.pairs.length * 2 - rows.length;
  const hasHeldOut = cohort.pairs.some(pair => pair.split === 'held_out');
  return {
    schema: 'ascension.jev-evaluation-report.v1', manifest_sha256: manifestDigest,
    evidence_kind: cohort.evidence_kind,
    evidence_status: 'source-derived',
    analysis_status: missing ? 'incomplete' : 'complete', unreported_results: missing,
    promotion_status: cohort.evidence_kind === 'synthetic' ? 'blocked_synthetic_evidence'
      : missing ? 'blocked_missing_results' : !hasHeldOut ? 'blocked_no_held_out_cohort'
        : 'requires_independent_runtime_and_statistical_review',
    caveats: [
      'Outcomes and pins are operator declarations; witness contents and native settlement are not verified.',
      'Recorded victories / scheduled is an observed fraction, not an imputation of missing results as defeats.',
      'Operational differences include timeout, infrastructure, refusal and not-started outcomes.',
      'Terminal-only comparisons can be selection-biased. No confidence interval or significance claim is made.',
      'Repeated seeds are not independent samples; the equal-seed summary weights each seed once.',
      'No provider or game was launched by this analyzer. No result automatically promotes a policy.',
    ],
    splits,
  };
}
