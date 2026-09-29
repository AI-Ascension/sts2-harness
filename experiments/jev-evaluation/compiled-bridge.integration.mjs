// SPDX-License-Identifier: MIT

// Real compiled Rust bridge + existing paired runner.
// Explicitly invoked with a built binary; a missing bridge is a failure, never a skipped test.
//
// SCOPE, narrowed by #299. This file used to stage a socket-free synthetic `--transport` beside
// the compiled binary and assert the runner's accounting of provider attempts across all eight
// response classes. The bridge no longer takes a transport: it performs the System One exchange
// itself against `api.typesafe.ai` with trust anchors compiled in from `webpki-roots`
// (ADR 0053). Host, port, and root store are compile-time constants with no injection seam, so a
// synthetic peer can no longer satisfy it -- which is the property that change was for. A local
// server presenting a substituted CA is refused by design, not by accident.
//
// What is kept here is the part that is genuinely provider-independent and still has value: that
// the REAL COMPILED BINARY is admitted by the runner's digest pin, and that it refuses, without
// contacting any provider, on the paths that refuse before an exchange. The nine cases that
// required a successful answer are retired rather than left failing; their response-class coverage
// lives in-process against bytes in `jev_tls_transport_tests.rs` (framing, status, chunked,
// close_notify) and `sts2_jev_bridge_tests.rs` (catalog, confidence, malformed envelope), driven
// end to end against a loopback TLS peer in `jev_tls_transport_loopback_tests.rs`.
//
// The honest gap this leaves: no automated lane exercises a real provider exchange end to end
// through the compiled binary. Closing that would require a test-only seam in production TLS code,
// which is a larger change than the coverage it buys and is not this file's call to make.
import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, copyFile, lstat, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { sha256 } from './contract.mjs';
import { executeRun, planRun } from './paired-runner.mjs';
import { fixture } from './runner-test-fixtures.mjs';

const binary = process.env.STS2_JEV_TEST_BRIDGE;
const revision = process.env.STS2_JEV_TEST_SOURCE_REVISION;
assert.notEqual(process.platform, 'win32', 'compiled capture integration requires Unix');
assert.ok(binary && isAbsolute(binary), 'STS2_JEV_TEST_BRIDGE must name an absolute compiled binary');
assert.match(revision ?? '', /^[0-9a-f]{40}$/, 'declare the actual build checkout revision');
assert.equal(await realpath(binary), binary, 'compiled binary locator must be canonical');
assert.ok((await lstat(binary)).isFile(), 'compiled binary must be a regular file');
const binaryDigest = sha256(await readFile(binary));
const bounded = { timeout: 45000 };
const json = async path => JSON.parse(await readFile(path, 'utf8'));

async function compiledFixture(t, mode = 'success') {
  const f = await fixture(t, mode);
  // Use the exact built bytes, including in a path with spaces; do not substitute a bridge oracle.
  await copyFile(binary, f.bridge); await chmod(f.bridge, 0o700);
  // The manifest schema still requires a `transport` entry and the runner still re-pins it, so the
  // file is staged and digested here. The compiled bridge is NOT given it and never reads it: since
  // #299 the exchange is in-process. Only the runner's two-artifact admission is under test, and
  // retiring that field is a separate schema change, not this lane's to make.
  const source = await readFile(new URL('./compiled-bridge-transport.mjs', import.meta.url));
  await writeFile(f.transport, Buffer.concat([Buffer.from(`#!${process.execPath}\n`), source]), { mode: 0o700 });
  f.manifest.bridge.sha256 = sha256(await readFile(f.bridge));
  assert.equal(f.manifest.bridge.sha256, binaryDigest);
  f.manifest.transport.sha256 = sha256(await readFile(f.transport));
  f.manifest.source_revision = revision;
  f.manifest.budgets.per_arm_timeout_ms = 5000;
  f.manifest.budgets.total_timeout_ms = 20000;
  Object.assign(f.input.observation.player, { max_hp: 80, energy: 3, gold: 0 });
  f.input.observation.state.turn_index = 2;
  await f.saveInput(); await f.save();
  return f;
}

async function execute(f) {
  return executeRun(f.path, (await planRun(f.path)).manifest_sha256, { environment: f.environment });
}

async function proofs(f) {
  return Promise.all((await readdir(f.proof)).map(name => json(join(f.proof, name))));
}

test('compiled single-action fast path records zero actual transport invocations', bounded, async t => {
  const f = await compiledFixture(t);
  f.input.legal_action_ids = ['only'];
  f.input.observation.legal_actions = [{ action_id: 'only', action: { kind: 'end_turn' } }];
  await f.saveInput(); await f.save();
  const result = await execute(f);
  assert.equal(result.counts.complete, 2); assert.equal(result.reserved_provider_attempts, 2);
  assert.equal(result.observed_provider_attempts_known_sum, 0);
  assert.deepEqual(await proofs(f), []);
  assert.equal((await json(join(f.manifest.output_directory, 'audit.json'))).counts.forced, 1);
});

test('compiled bridge rejects non-ASCII action identifiers before transport invocation', bounded, async t => {
  const f = await compiledFixture(t);
  f.input.legal_action_ids = ['action-😀', 'action-\uE000'];
  await f.saveInput(); await f.save();
  const result = await execute(f);
  assert.equal(result.counts.bridge_failed, 2); assert.equal(result.incomplete, true);
  assert.equal(result.observed_provider_attempts_known_sum, 0);
  assert.deepEqual(await proofs(f), []);
  const audit = await json(join(f.manifest.output_directory, 'audit.json'));
  assert.equal(audit.counts.failed, 1); assert.equal(audit.independent_action_pairs, 0);
});
