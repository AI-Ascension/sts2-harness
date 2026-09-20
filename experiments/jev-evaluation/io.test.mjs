// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { sha256 } from './contract.mjs';
import { jsonFile, localRecord, readBounded, resultsFile } from './io.mjs';
import { baselineRecord, cohort, hash, pairManifest, tacticalRecord, PRIVATE_MARKER } from './test-fixtures.mjs';

const cli = fileURLToPath(new URL('./cli.mjs', import.meta.url));
async function scratch(context) {
  const root = fileURLToPath(new URL('../../target/jev-evaluation-tests/', import.meta.url));
  await mkdir(root, { recursive: true });
  const path = await mkdtemp(join(root, 'fixture-'));
  context.after(() => rm(path, { recursive: true, force: true }));
  return path;
}
const run = args => spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8', timeout: 5000 });

test('bounded reader hashes exact bytes and refuses oversized files', async context => {
  const root = await scratch(context); const file = join(root, 'data.json');
  const text = '{ "a": 1 }\n'; await writeFile(file, text);
  assert.equal((await jsonFile(file)).sha256, sha256(text));
  await assert.rejects(() => readBounded(file, 3), /file_size_bound/);
});


test('missing file errors never echo private paths', async context => {
  const root = await scratch(context);
  await assert.rejects(() => jsonFile(join(root, PRIVATE_MARKER)), /^ValidationError: file_unavailable$/);
});

test('record byte-digest mismatches are refused', async context => {
  const root = await scratch(context); await writeFile(join(root, 'record.json'), '{}');
  await assert.rejects(() => localRecord(join(root, 'pairs.json'), { path: 'record.json', sha256: hash('wrong') }),
    /record_digest_mismatch/);
});

for (const path of ['../outside.json', '/absolute.json', 'C:/private.json', 'nested/../../other.json', 'nested\\other.json']) {
  test(`record path shape ${path} is refused`, async context => {
    const root = await scratch(context);
    await assert.rejects(() => localRecord(join(root, 'pairs.json'), { path, sha256: hash('record') }), /record_path/);
  });
}

test('record symlinks cannot escape the manifest directory', async context => {
  const root = await scratch(context); await mkdir(join(root, 'nested'));
  await writeFile(join(root, 'outside.json'), '{}');
  try { await symlink(join(root, 'outside.json'), join(root, 'nested', 'link.json')); }
  catch (error) { if (error.code === 'EPERM') { context.skip('platform denies symlink creation'); return; } throw error; }
  await assert.rejects(() => localRecord(join(root, 'nested', 'pairs.json'),
    { path: 'link.json', sha256: sha256('{}') }), /record_path_escape/);
});

test('JSONL input rejects malformed UTF-8 instead of replacing bytes', async context => {
  const root = await scratch(context); const path = join(root, 'rows.jsonl');
  await writeFile(path, Buffer.from([123, 34, 120, 34, 58, 34, 255, 34, 125]));
  await assert.rejects(() => resultsFile(path), /invalid_utf8/);
});

test('CLI plan is read-only and reports a raw-file manifest digest', async context => {
  const root = await scratch(context); const path = join(root, 'cohort.json');
  const text = JSON.stringify(cohort()); await writeFile(path, text);
  const outcome = run(['plan', path]);
  assert.equal(outcome.status, 0, outcome.stderr);
  const report = JSON.parse(outcome.stdout);
  assert.equal(report.manifest_sha256, sha256(text));
  assert.equal(report.launches_performed, 0);
});

test('CLI returns exit 3 plus a report for incomplete cohorts', async context => {
  const root = await scratch(context); const path = join(root, 'cohort.json');
  await writeFile(path, JSON.stringify(cohort())); await writeFile(join(root, 'empty.jsonl'), '');
  const outcome = run(['compare', path, join(root, 'empty.jsonl')]);
  assert.equal(outcome.status, 3, outcome.stderr);
  assert.equal(JSON.parse(outcome.stdout).unreported_results, 4);
});

test('CLI imports actual file bytes and exports only the audit', async context => {
  const root = await scratch(context); const manifest = pairManifest();
  for (const [arm, record] of [['baseline', baselineRecord()], ['tactical', tacticalRecord()]]) {
    const text = JSON.stringify(record); await writeFile(join(root, `${arm}.json`), text);
    manifest.pairs[0][arm].sha256 = sha256(text);
  }
  const path = join(root, 'pairs.json'); await writeFile(path, JSON.stringify(manifest));
  const outcome = run(['audit', path]);
  assert.equal(outcome.status, 0, outcome.stderr);
  assert.equal(JSON.parse(outcome.stdout).independent_action_agreement.agreements, 1);
  assert.equal(outcome.stdout.includes(PRIVATE_MARKER), false);
});

test('CLI invalid-input failures emit neither stack traces nor raw input', async context => {
  const root = await scratch(context); const path = join(root, PRIVATE_MARKER);
  await writeFile(path, `{private:${PRIVATE_MARKER}}`);
  const outcome = run(['audit', path]);
  assert.equal(outcome.status, 2);
  assert.equal(outcome.stdout, '');
  assert.equal(outcome.stderr.trim(), 'invalid_json');
});

test('demo generates explicitly synthetic reports and refuses to overwrite them', async context => {
  const root = await scratch(context); const output = join(root, 'new-demo');
  const script = fileURLToPath(new URL('./demo.mjs', import.meta.url));
  const first = spawnSync(process.execPath, [script, output], { encoding: 'utf8', timeout: 5000 });
  assert.equal(first.status, 0, first.stderr);
  const report = await jsonFile(join(output, 'outcome-report.json'));
  assert.equal(report.value.evidence_kind, 'synthetic');
  assert.equal(report.value.promotion_status, 'blocked_synthetic_evidence');
  const second = spawnSync(process.execPath, [script, output], { encoding: 'utf8', timeout: 5000 });
  assert.equal(second.status, 2);
  assert.equal((await jsonFile(join(output, 'outcome-report.json'))).sha256, report.sha256);
});
