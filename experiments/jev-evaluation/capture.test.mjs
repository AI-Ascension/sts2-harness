// SPDX-License-Identifier: MIT

// Synthetic contract fixtures shared with a Rust projection test, not observed provider results.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { sha256 } from './contract.mjs';
import { CAPTURE_PAIRS_SCHEMA, modelFingerprint, validateCapture } from './capture-records.mjs';
import { auditCaptures, validateCaptureManifest } from './capture-audit.mjs';

const golden = JSON.parse(readFileSync(new URL(
  '../../crates/harness/src/bin/support/jev_capture_golden.json', import.meta.url), 'utf8'));
const copy = value => structuredClone(value);
const hex = letter => letter.repeat(64);

function fixtures() {
  const baseline = copy(golden);
  const tactical = copy(golden);
  tactical.profile = 'jev-tactical-v1';
  tactical.model_execution_id_digest = hex('b');
  tactical.provider.request_digest = hex('c');
  tactical.provider.question_set_digest = hex('d');
  tactical.decision.selected_index = 1;
  tactical.tactical = { applied: true, within_request_index: 0,
    minimum_evidence: 0.8, minimum_margin: 0.1, minimum_safety: 0.5,
    rows: [1, 0].map(index => ({ index, scores: Array(6).fill(index),
      evidence: 0.95, min_confidence: 0.9, utility: index })) };
  const manifest = { schema: CAPTURE_PAIRS_SCHEMA, model: 'jev-1.13.0', bridge_digest: hex('a'),
    pairs: [{ pair_id: 'synthetic-pair', baseline: { path: 'b.json', sha256: hex('e') },
      tactical: { path: 't.json', sha256: hex('f') } }] };
  return { baseline, tactical, manifest };
}

async function audit(fixture = fixtures()) {
  return auditCaptures(fixture.manifest, async descriptor =>
    descriptor.path === 'b.json' ? fixture.baseline : fixture.tactical);
}

test('redacted model fingerprint matches the shared Rust golden', () => {
  assert.equal(modelFingerprint('jev-1.13.0'), golden.requested_model_digest);
  assert.equal(validateCapture(copy(golden)).decision.selected_index, 0);
});

test('independent decisions and within-request decisions have separate denominators', async () => {
  const report = await audit();
  assert.equal(report.counts.disagree, 1);
  assert.equal(report.independent_action_pairs, 1);
  assert.equal(report.metrics.within_request_action_pairs, 1);
  assert.equal(report.metrics.within_request_action_agree, 0);
  assert.equal(report.metrics.provider_attempts_known_sum, 2);
  assert.equal(report.gameplay_improvement_established, false);
});

test('independent agreement does not turn within-request disagreement into agreement', async () => {
  const fixture = fixtures();
  fixture.baseline.decision.selected_index = 1;
  const report = await audit(fixture);
  assert.equal(report.counts.agree, 1);
  assert.equal(report.metrics.within_request_action_agree, 0);
});

test('report exports no content, identities, action indices or fingerprints', async () => {
  const report = JSON.stringify(await audit());
  for (const forbidden of ['synthetic-pair', 'jev-1.13.0', 'b.json', 't.json',
    'selected_index', golden.input_digest, golden.model_execution_id_digest]) {
    assert.equal(report.includes(forbidden), false, forbidden);
  }
});

for (const [name, mutate, expected] of [
  ['unreported', f => { f.manifest.pairs[0].tactical = null; }, 'unreported'],
  ['source drift', f => { f.tactical.bridge_digest = hex('e'); }, 'pin_mismatch'],
  ['requested model drift', f => { f.tactical.requested_model_digest = hex('e'); }, 'pin_mismatch'],
  ['returned model drift', f => { f.tactical.provider.response_model_digest = hex('e'); }, 'model_drift'],
  ['missing returned model', f => { f.tactical.provider.response_model_digest = null; }, 'model_identity_unverified'],
  ['changed input', f => { f.tactical.input_digest = hex('e'); }, 'input_mismatch'],
  ['changed catalog', f => { f.tactical.catalog_digest = hex('e'); }, 'catalog_mismatch'],
  ['changed gate', f => { f.tactical.confidence_gate = 0.3; }, 'gate_mismatch'],
  ['changed normalized request', f => { f.tactical.provider.shared_request_digest = hex('e'); }, 'shared_request_mismatch'],
]) {
  test(`audit keeps ${name} out of the agreement denominator`, async () => {
    const fixture = fixtures(); mutate(fixture);
    const result = await audit(fixture);
    assert.equal(result.counts[expected], 1);
    assert.equal(result.independent_action_pairs, 0);
    assert.equal(result.independent_action_agreement, null);
    assert.equal(result.incomplete, true);
  });
}

test('pending and failed captures retain unknown measurements and attempts', async () => {
  for (const status of ['pending', 'failed']) {
    const fixture = fixtures();
    const item = fixture.tactical;
    item.status = status;
    delete item.decision; delete item.provider; delete item.tactical;
    if (status === 'pending') { item.provider_attempts = null; item.elapsed_ms = null; }
    const report = await audit(fixture);
    assert.equal(report.counts[status], 1);
    assert.equal(report.metrics.input_tokens_unknown_records, 1);
    assert.equal(report.metrics.provider_attempts_known_sum, status === 'pending' ? 1 : 2);
  }
});

test('all missing outcomes remain scheduled and unknown', async () => {
  const fixture = fixtures();
  fixture.manifest.pairs[0].baseline = null; fixture.manifest.pairs[0].tactical = null;
  const report = await audit(fixture);
  assert.equal(report.scheduled_pairs, 1);
  assert.equal(report.counts.unreported, 1);
  assert.equal(report.metrics.provider_attempts_unknown_records, 2);
  assert.equal(report.metrics.input_tokens_unknown_records, 2);
});

test('refusals and low-evidence estimates remain diagnostics, not defeats', async () => {
  const fixture = fixtures();
  fixture.tactical.tactical.rows[1].evidence = 0.1;
  fixture.tactical.decision = { kind: 'reobserve', selected_index: null, candidate_index: null };
  const report = await audit(fixture);
  assert.equal(report.counts.refusal, 1);
  assert.equal(report.metrics.low_evidence_rows, 1);
  assert.equal(report.independent_action_pairs, 0);
});

test('tactical fallback and forced captures are not promoted to tactical evidence', async () => {
  const fixture = fixtures();
  fixture.tactical.tactical = { applied: false, fallback_reason: 'question_batch_budget' };
  assert.equal((await audit(fixture)).counts.tactical_fallback, 1);
  fixture.tactical.tactical.fallback_reason = 'legacy_forced_group';
  fixture.tactical.provider_attempts = 0;
  fixture.tactical.provider = null;
  const report = await audit(fixture);
  assert.equal(report.counts.forced, 1);
  assert.equal(report.metrics.provider_attempts_known_sum, 1);
});

for (const [name, mutate] of [
  ['unknown header', c => { c.prompt = 'PRIVATE'; }],
  ['raw rationale', c => { c.decision.rationale = 'PRIVATE'; }],
  ['raw state', c => { c.provider.state = 'PRIVATE'; }],
  ['oversize catalog', c => { c.catalog_count = 257; }],
  ['zero catalog', c => { c.catalog_count = 0; }],
  ['unsafe integer', c => { c.elapsed_ms = Number.MAX_SAFE_INTEGER + 1; }],
  ['negative duration', c => { c.elapsed_ms = -1; }],
  ['second inference', c => { c.provider_attempts = 2; }],
  ['non-finite gate', c => { c.confidence_gate = NaN; }],
  ['bad fingerprint', c => { c.input_digest = 'secret'; }],
  ['unknown status', c => { c.status = 'victory'; }],
  ['missing provider', c => { c.provider = null; }],
  ['unbounded index', c => { c.decision.selected_index = 2; }],
  ['unknown decision', c => { c.decision.kind = 'victory'; }],
  ['raw fallback text', c => { c.tactical = { applied: false, fallback_reason: 'SECRET' }; }],
  ['duplicate score row', c => { c.tactical.rows[1].index = 1; }],
  ['missing score row', c => { c.tactical.rows.pop(); }],
  ['invalid score', c => { c.tactical.rows[0].scores[0] = Infinity; }],
  ['forged utility', c => { c.tactical.rows[0].utility = 0.5; }],
  ['changed threshold', c => { c.tactical.minimum_evidence = 0.1; }],
  ['wrong ranking', c => { c.tactical.rows.reverse(); }],
  ['wrong winner', c => { c.decision.selected_index = 0; }],
  ['forced tactical refusal', c => {
    c.tactical.rows[1].evidence = 0.1;
    c.decision = { kind: 'reobserve', selected_index: null, candidate_index: 0 };
  }],
]) {
  test(`capture schema rejects ${name}`, () => {
    const value = fixtures().tactical;
    mutate(value);
    assert.throws(() => validateCapture(value));
  });
}

test('reused executions and duplicate pairs are rejected', async () => {
  const fixture = fixtures();
  fixture.tactical.model_execution_id_digest = fixture.baseline.model_execution_id_digest;
  await assert.rejects(audit(fixture), /capture_reused_execution/);
  fixture.manifest.pairs.push(copy(fixture.manifest.pairs[0]));
  assert.throws(() => validateCaptureManifest(fixture.manifest), /capture_duplicate_pair/);
});

test('floating model aliases cannot be a comparison pin', () => {
  const fixture = fixtures();
  fixture.manifest.model = 'jev-latest';
  assert.throws(() => validateCaptureManifest(fixture.manifest), /floating_model/);
});

test('a mislabelled available arm is rejected even when its partner is unreported', async () => {
  const fixture = fixtures();
  fixture.baseline = fixture.tactical;
  fixture.manifest.pairs[0].tactical = null;
  await assert.rejects(audit(fixture), /capture_arm/);
});

test('CLI loads byte-pinned local captures and preserves exit semantics', async t => {
  const parent = resolve('target/jev-capture-cli-tests');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'fixture-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const fixture = fixtures();
  for (const [arm, file] of [['baseline', 'b.json'], ['tactical', 't.json']]) {
    const bytes = Buffer.from(`${JSON.stringify(fixture[arm])}\n`);
    await writeFile(join(root, file), bytes);
    fixture.manifest.pairs[0][arm].sha256 = sha256(bytes);
  }
  const manifest = join(root, 'pairs.json');
  await writeFile(manifest, JSON.stringify(fixture.manifest));
  const cli = fileURLToPath(new URL('./capture-cli.mjs', import.meta.url));
  let result = spawnSync(process.execPath, [cli, manifest], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).counts.disagree, 1);
  fixture.manifest.pairs[0].tactical = null;
  await writeFile(manifest, JSON.stringify(fixture.manifest));
  result = spawnSync(process.execPath, [cli, manifest], { encoding: 'utf8' });
  assert.equal(result.status, 3);
  await writeFile(join(root, 'b.json'), 'PRIVATE_MALFORMED_INPUT');
  result = spawnSync(process.execPath, [cli, manifest], { encoding: 'utf8' });
  assert.equal(result.status, 2);
  assert.equal(result.stdout, '');
  assert.equal(result.stderr.includes('PRIVATE'), false);
  assert.equal(result.stderr.includes(root), false);
  assert.equal((await readFile(join(root, 'b.json'), 'utf8')), 'PRIVATE_MALFORMED_INPUT');
});