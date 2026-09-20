// SPDX-License-Identifier: MIT

// Original MIT synthetic process oracle. No sockets, provider access, host files or game behavior.
// Tests install these bytes with an absolute Node shebang into a private executable fixture.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, basename, join } from 'node:path';
import { createHash } from 'node:crypto';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sorted = value => Array.isArray(value) ? value.map(sorted)
  : value !== null && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sorted(value[key])])) : value;
const fp = (domain, value) => hash(`ascension.jev-capture.v1/${domain}\0${JSON.stringify(sorted(value))}`);
const args = process.argv.slice(2), flags = Object.create(null);
for (let at = 0; at < args.length; at++) {
  if (args[at] === '--tactical') flags.tactical = true;
  else flags[args[at].slice(2)] = args[++at];
}
const directory = flags['audit-dir'], slot = basename(directory), runRoot = dirname(directory);
if (!existsSync(join(runRoot, `${slot}.pending.json`)) || !existsSync(join(runRoot, 'run.pending.json'))) {
  process.exit(54);
}
const input = JSON.parse(readFileSync(0, 'utf8'));
const proofRoot = process.env.JEV_RUNNER_FIXTURE_PROOF;
if (proofRoot) writeFileSync(join(proofRoot, `${slot}.json`), JSON.stringify({ input, args,
  environment_names: Object.keys(process.env), secret_present: process.env.TYPESAFE_API_KEY === 'synthetic-secret' }),
{ mode: 0o600, flag: 'wx' });
const mode = process.env.JEV_RUNNER_FIXTURE_CASE ?? 'success';
const catalog = [...input.legal_action_ids].sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
const comparable = { ...input }; delete comparable.model_execution_id;
const header = { schema: 'ascension.jev-redacted-capture.v1', profile: flags.tactical ? 'jev-tactical-v1' : 'baseline',
  bridge_digest: hash(readFileSync(process.argv[1])), model_execution_id_digest: fp('model_execution_id', input.model_execution_id),
  input_digest: fp('bridge_input_without_execution_id', comparable), catalog_digest: fp('sorted_host_catalog', catalog),
  catalog_count: catalog.length, requested_model_digest: fp('model', flags.model), confidence_gate: Number(flags.gate) / 100 };
if (mode === 'bad_identity') header.model_execution_id_digest = 'f'.repeat(64);
if (mode === 'input_drift' && flags.tactical) header.input_digest = 'a'.repeat(64);
const write = (kind, value) => writeFileSync(join(directory, `attempt-0000.${kind}.json`),
  `${JSON.stringify(value)}\n`, { flag: 'wx', mode: 0o600 });
if (mode !== 'no_capture') write('pending', { ...header, status: 'pending', provider_attempts: null, elapsed_ms: null });
if (mode === 'timeout') { setInterval(() => {}, 1000); await new Promise(() => {}); }
if (mode === 'flood') { process.stdout.write('s'.repeat(9000)); process.exit(0); }
if (mode === 'stderr_failure') { process.stderr.write('synthetic-secret /private/raw-output\n'); process.exit(2); }
if (mode === 'capture_failed') { write('result', { ...header, status: 'failed', provider_attempts: 1, elapsed_ms: 4 }); process.exit(2); }
const called = catalog.length !== 1;
let selected = flags.tactical && called ? 1 : 0;
const lowEvidence = mode === 'low_evidence' && flags.tactical && called;
if (lowEvidence) selected = null;
const decision = { kind: lowEvidence ? 'reobserve' : 'action', selected_index: selected, candidate_index: null };
const result = { ...header, status: 'complete', provider_attempts: Number(called), elapsed_ms: 5,
  decision, provider: called ? { request_digest: hash(flags.tactical ? 'tactical' : 'baseline'),
    shared_request_digest: hash(JSON.stringify(sorted(comparable))), question_set_digest: hash('questions'),
    response_model_digest: mode === 'model_drift' ? 'e'.repeat(64) : fp('model', flags.model), input_tokens: 100 } : null,
  tactical: flags.tactical ? called ? { applied: true, within_request_index: 0,
    minimum_evidence: 0.8, minimum_margin: 0.1, minimum_safety: 0.5,
    rows: catalog.map((_, index) => ({ index, scores: Array(6).fill(index === 1 ? 1 : 0),
      evidence: lowEvidence ? 0.1 : 0.95, min_confidence: 0.9, utility: index === 1 ? 1 : 0 }))
      .sort((a, b) => b.utility - a.utility || a.index - b.index) }
    : { applied: false, fallback_reason: 'forced_action' } : null };
if (mode !== 'no_capture') write('result', result);
if (mode === 'extra_file') writeFileSync(join(directory, 'unexpected.json'), '{}', { mode: 0o600 });
if (mode === 'exit_after_complete') process.exit(3);
if (mode === 'invalid_stdout') { process.stdout.write('{ invalid synthetic-secret'); process.exit(0); }
if (mode === 'mutate_transport') writeFileSync(flags.transport, 'changed synthetic transport');
if (mode === 'bad_stdout') selected = catalog.length > 1 ? (selected === 0 ? 1 : 0) : null;
process.stdout.write(JSON.stringify(lowEvidence ? { decision: 'reobserve', rationale: 'synthetic refusal' }
  : { decision: 'action', action_id: selected === null ? 'out-of-catalog' : catalog[selected],
    confidence: 90, rationale: 'synthetic decision, not a provider result' }));
