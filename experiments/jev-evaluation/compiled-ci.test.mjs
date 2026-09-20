// SPDX-License-Identifier: MIT

// Test the shell entrypoint with original, local command doubles. These tests do not
// compile Rust, run the real integration suite, invoke a provider, or establish gameplay.
import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const script = fileURLToPath(new URL('./compiled-ci.sh', import.meta.url));
const unix = { skip: process.platform === 'win32' };
const revision = 'a'.repeat(40);

// One test-owned executable source dispatches by basename; nothing here is a game fixture.
const command = `#!${process.execPath}
import { appendFileSync, chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
const name = basename(process.argv[1]), args = process.argv.slice(2);
const root = process.env.GATE_FIXTURE_ROOT, mode = process.env.GATE_FIXTURE_MODE;
appendFileSync(process.env.GATE_FIXTURE_LOG, JSON.stringify({ name, args, cwd: process.cwd(),
  target: process.env.CARGO_TARGET_DIR, jobs: process.env.CARGO_BUILD_JOBS,
  debug: process.env.CARGO_PROFILE_DEV_DEBUG, incremental: process.env.CARGO_INCREMENTAL,
  binary: process.env.STS2_JEV_TEST_BRIDGE, revision: process.env.STS2_JEV_TEST_SOURCE_REVISION }) + '\\n');
if (name === 'git') {
  if (mode === 'git_failure') process.exit(41);
  if (args.join(' ') === 'rev-parse --show-toplevel') {
    console.log(mode === 'relative_root' ? 'relative' : root);
  } else if (args.join(' ') === 'rev-parse --verify HEAD') {
    const counter = join(root, 'revision-reads');
    const count = existsSync(counter) ? Number(readFileSync(counter, 'utf8')) + 1 : 1;
    writeFileSync(counter, String(count));
    console.log(mode === 'bad_revision' ? 'invalid' :
      mode === 'head_changed' && count > 1 ? 'b'.repeat(40) : 'a'.repeat(40));
  } else process.exit(42);
} else if (name === 'cargo') {
  if (mode === 'cargo_failure') process.exit(17);
  if (mode === 'missing_binary') process.exit(0);
  const directory = join(process.env.CARGO_TARGET_DIR, 'debug');
  mkdirSync(directory, { recursive: true });
  const binary = join(directory, 'sts2-jev-bridge');
  if (mode === 'directory_binary') mkdirSync(binary);
  else { writeFileSync(binary, '#!/bin/false\\n'); chmodSync(binary, mode === 'not_executable' ? 0o600 : 0o700); }
} else if (name === 'realpath') {
  process.exit(19);
} else if (name === 'node') {
  if (mode === 'tests_failure') process.exit(23);
} else process.exit(43);
`;

async function fixture(t, mode = 'success') {
  const base = fileURLToPath(new URL('../../target/jev-compiled-ci-tests/', import.meta.url));
  await mkdir(base, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(base, 'case-'));
  await chmod(root, 0o700);
  t.after(() => rm(root, { recursive: true, force: true }));
  const checkout = join(root, 'checkout with spaces'), bin = join(root, 'commands');
  await mkdir(checkout, { mode: 0o700 }); await mkdir(bin, { mode: 0o700 });
  await writeFile(join(bin, 'package.json'), '{"type":"module"}\n', { mode: 0o600 });
  for (const name of ['git', 'cargo', 'node']) await writeFile(join(bin, name), command, { mode: 0o700 });
  if (mode === 'realpath_failure') await writeFile(join(bin, 'realpath'), command, { mode: 0o700 });
  const log = join(root, 'calls.jsonl');
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`,
    GATE_FIXTURE_ROOT: checkout, GATE_FIXTURE_MODE: mode, GATE_FIXTURE_LOG: log };
  const run = (args = [], extra = {}) => spawnSync('/bin/bash', [script, ...args], {
    cwd: checkout, env: { ...env, ...extra }, encoding: 'utf8', timeout: 10000,
  });
  const calls = async () => {
    if (!(await readdir(root)).includes('calls.jsonl')) return [];
    return (await readFile(log, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
  };
  return { checkout, run, calls };
}

test('compiled gate builds a locked pinned bridge and invokes the actual integration filename', unix, async t => {
  const f = await fixture(t), result = f.run();
  assert.equal(result.status, 0, result.stderr);
  const calls = await f.calls();
  assert.deepEqual(calls.map(call => call.name), ['git', 'git', 'cargo', 'node', 'git']);
  assert.deepEqual(calls[2].args, ['+1.97.1', 'build', '--locked', '--package', 'sts2-harness', '--bin', 'sts2-jev-bridge']);
  assert.deepEqual(calls[3].args, ['--test', 'experiments/jev-evaluation/compiled-bridge.integration.mjs']);
  assert.equal(calls[3].cwd, f.checkout);
  assert.equal(calls[3].binary, join(f.checkout, 'target/jev-compiled-ci/debug/sts2-jev-bridge'));
  assert.equal(calls[3].revision, revision);
  assert.equal(calls[2].jobs, '2'); assert.equal(calls[2].debug, '0'); assert.equal(calls[2].incremental, '0');
});

test('ambient Cargo output and bridge identities cannot substitute a different binary', unix, async t => {
  const f = await fixture(t);
  const result = f.run([], { CARGO_TARGET_DIR: join(f.checkout, 'ambient output'),
    STS2_JEV_TEST_BRIDGE: join(f.checkout, 'ambient bridge'),
    STS2_JEV_TEST_SOURCE_REVISION: 'f'.repeat(40), CARGO_BUILD_JOBS: '99', CARGO_INCREMENTAL: '1' });
  assert.equal(result.status, 0, result.stderr);
  const calls = await f.calls(), build = calls.find(call => call.name === 'cargo'), suite = calls.find(call => call.name === 'node');
  assert.equal(build.target, join(f.checkout, 'target/jev-compiled-ci'));
  assert.equal(build.jobs, '2'); assert.equal(build.incremental, '0');
  assert.equal(suite.revision, revision);
  assert.equal(suite.binary, join(f.checkout, 'target/jev-compiled-ci/debug/sts2-jev-bridge'));
});

for (const [mode, status, names] of [
  ['git_failure', 41, ['git']],
  ['relative_root', 2, ['git']],
  ['bad_revision', 2, ['git', 'git']],
  ['cargo_failure', 17, ['git', 'git', 'cargo']],
  ['missing_binary', 1, ['git', 'git', 'cargo']],
  ['directory_binary', 1, ['git', 'git', 'cargo']],
  ['not_executable', 1, ['git', 'git', 'cargo']],
  ['realpath_failure', 19, ['git', 'git', 'cargo', 'realpath']],
  ['tests_failure', 23, ['git', 'git', 'cargo', 'node']],
  ['head_changed', 1, ['git', 'git', 'cargo', 'node', 'git']],
]) {
  test(`${mode} fails the gate without retrying or running later stages`, unix, async t => {
    const f = await fixture(t, mode), result = f.run();
    assert.equal(result.status, status, result.stderr);
    assert.deepEqual((await f.calls()).map(call => call.name), names);
  });
}

test('unknown arguments are rejected without invoking build or test commands', unix, async t => {
  const f = await fixture(t), result = f.run(['--bridge', '/another-binary']);
  assert.equal(result.status, 2); assert.deepEqual(await f.calls(), []);
});
