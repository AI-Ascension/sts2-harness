// SPDX-License-Identifier: MIT

// Descriptive diagnostics over validated journals/captures, not rewards, win odds or causal effects.
import { integer } from './contract.mjs';
import { summarize } from './runner-journal.mjs';

const rate = (numerator, denominator) => ({ numerator, denominator,
  value: denominator === 0 ? null : numerator / denominator });

export function measurements(values, signed = false) {
  const known = values.filter(value => value !== null).sort((a, b) => a - b);
  const total = known.reduce((sum, value) => {
    const next = sum + value;
    integer(signed ? Math.abs(next) : next);
    return next;
  }, 0);
  const middle = Math.floor(known.length / 2);
  return { known_count: known.length, unknown_count: values.length - known.length, known_sum: total,
    mean_known: known.length === 0 ? null : total / known.length,
    median_known: known.length === 0 ? null : known.length % 2 ? known[middle]
      : known[middle - 1] / 2 + known[middle] / 2,
    minimum_known: known[0] ?? null, maximum_known: known.at(-1) ?? null };
}

function gates(records) {
  const applied = records.filter(record => record?.status === 'complete' && record.tactical?.applied);
  const counts = { evaluated_decisions: applied.length, refused_decisions: 0, assessed_rows: 0,
    low_evidence_rows: 0, low_evidence_decisions: 0, low_confidence_decisions: 0,
    low_safety_decisions: 0, low_margin_decisions: 0, reasons_can_overlap: true };
  for (const record of applied) {
    const t = record.tactical, [first, second] = t.rows;
    const lowEvidence = t.rows.filter(row => row.evidence < t.minimum_evidence).length;
    counts.assessed_rows += t.rows.length;
    counts.low_evidence_rows += lowEvidence;
    counts.refused_decisions += Number(record.decision.kind === 'reobserve');
    counts.low_evidence_decisions += Number(lowEvidence > 0);
    counts.low_confidence_decisions += Number(first.min_confidence < record.confidence_gate);
    counts.low_safety_decisions += Number(first.scores[5] < t.minimum_safety);
    counts.low_margin_decisions += Number(first.utility - second.utility < t.minimum_margin);
  }
  return counts;
}

function armDiagnostics(rows, records) {
  const complete = records.filter(record => record?.status === 'complete');
  const actions = complete.filter(record => record.decision.kind === 'action').length;
  const refusals = complete.length - actions;
  const fallbackReasons = {};
  for (const record of complete) {
    if (record.tactical?.applied === false) {
      const key = record.tactical.fallback_reason;
      fallbackReasons[key] = (fallbackReasons[key] ?? 0) + 1;
    }
  }
  return { scheduled_arms: rows.length, complete_captures: complete.length,
    captures_missing_or_incomplete: rows.length - complete.length,
    actions_over_scheduled: rate(actions, rows.length),
    refusals_over_scheduled: rate(refusals, rows.length),
    refusals_over_complete_captures: rate(refusals, complete.length),
    forced_actions: complete.filter(record => record.provider_attempts === 0).length,
    tactical_fallback_reasons: fallbackReasons, tactical_gates: gates(records),
    measurements: Object.fromEntries(['observed_provider_attempts', 'input_tokens',
      'elapsed_ms', 'capture_elapsed_ms'].map(key => [key, measurements(rows.map(row => row[key]))])) };
}

function pairedMeasurements(plan, rows, audit) {
  // Include refusals, rather than estimating cost only on decisions that succeeded.
  // Drift, missing arms, forced actions and tactical fallbacks are not matched provider work.
  const valid = new Set(audit.outcomes.filter(outcome =>
    ['agree', 'disagree', 'refusal'].includes(outcome.status)).map(outcome => outcome.pair_position));
  const pairs = [];
  for (let index = 0; index < rows.length; index += 2) {
    if (!valid.has(index / 2)) continue;
    const first = plan.scheduled[index].arm;
    pairs.push(first === 'baseline' ? [rows[index], rows[index + 1]] : [rows[index + 1], rows[index]]);
  }
  return { direction: 'tactical_minus_baseline', includes_refusals: true,
    eligible_pairs: pairs.length, excluded_pairs: rows.length / 2 - pairs.length,
    metrics: Object.fromEntries(['input_tokens', 'elapsed_ms', 'capture_elapsed_ms'].map(key => [key,
      measurements(pairs.map(([a, b]) => a[key] === null || b[key] === null ? null : b[key] - a[key]), true)])) };
}

export function diagnosticGroup(plan, rows, records, audit) {
  return { execution: summarize(plan, rows, 'read_only_diagnostics_not_liveness_proof'),
    audit, by_arm: Object.fromEntries(['baseline', 'tactical'].map(arm => {
      const indices = plan.scheduled.flatMap((entry, index) => entry.arm === arm ? [index] : []);
      return [arm, armDiagnostics(indices.map(index => rows[index]), indices.map(index => records[index]))];
    })), paired_measurements: pairedMeasurements(plan, rows, audit) };
}
