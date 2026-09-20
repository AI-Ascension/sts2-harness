// SPDX-License-Identifier: MIT

// Hand-authored synthetic fixtures only. Raw proof files here are tests, never production outputs.
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { sha256 } from './contract.mjs';
import { RUNNER_SCHEMA } from './runner-contract.mjs';

export async function fixture(t, mode = 'success') {
  const base = fileURLToPath(new URL('../../target/jev-runner-tests/', import.meta.url));
  await mkdir(base, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(base, 'case-'));
  await chmod(root, 0o700); // A test workspace may inherit setgid; the capture contract is exact 0700.
  t.after(() => rm(root, { recursive: true, force: true }));
  const bin = join(root, 'Program Files'), proof = join(root, 'proof');
  await mkdir(bin, { mode: 0o700 }); await mkdir(proof, { mode: 0o700 });
  const bridge = join(bin, 'synthetic bridge'), transport = join(bin, 'System One transport');
  const source = await readFile(new URL('./runner-fake-bridge.mjs', import.meta.url));
  await writeFile(bridge, Buffer.concat([Buffer.from(`#!${process.execPath}\n`), source]), { mode: 0o700 });
  await writeFile(transport, '#!/bin/false\n', { mode: 0o700 });
  const input = { model_execution_id: 'approved-original', objective: 'synthetic objective private-marker',
    hard_constraints: [], legal_action_ids: ['action-b', 'action-a'], observation: {
      state_id: 'synthetic-state', generation: 1, player: { hp: 30, hand: [] }, state: { state: 'combat' } } };
  const inputPath = join(root, 'approved input.json'), path = join(root, 'manifest.json');
  const manifest = { schema: RUNNER_SCHEMA, experiment_id: 'synthetic-experiment', evidence_kind: 'synthetic',
    source_revision: '87d7e991bde6ca4c51a397a6af47ae46b3a2d9d4', model: 'jev-1.13.0',
    bridge: { path: bridge, sha256: sha256(await readFile(bridge)) },
    transport: { path: transport, sha256: sha256(await readFile(transport)) },
    inherited_environment: ['JEV_RUNNER_FIXTURE_CASE', 'JEV_RUNNER_FIXTURE_PROOF', 'TYPESAFE_API_KEY'],
    confidence_gate_percent: 20, output_directory: join(root, 'run'),
    budgets: { max_pairs: 4, max_provider_attempts: 8, per_arm_timeout_ms: 2000,
      total_timeout_ms: 12000, max_total_input_bytes: 524288 },
    pairs: [{ pair_id: 'pair-0001', input_path: 'approved input.json', input_sha256: '',
      cluster_sha256: 'b'.repeat(64), split: 'held_out', repetition: 0 }] };
  const saveInput = async () => {
    const bytes = Buffer.from(`${JSON.stringify(input)}\n`);
    await writeFile(inputPath, bytes, { mode: 0o600 });
    manifest.pairs[0].input_sha256 = sha256(bytes);
  };
  const save = async () => { await writeFile(path, `${JSON.stringify(manifest)}\n`, { mode: 0o600 }); };
  await saveInput(); await save();
  return { root, path, manifest, input, inputPath, bridge, transport, proof, save, saveInput,
    environment: { JEV_RUNNER_FIXTURE_CASE: mode, JEV_RUNNER_FIXTURE_PROOF: proof,
      TYPESAFE_API_KEY: 'synthetic-secret', UNDECLARED_SECRET: 'must-not-inherit' },
    chmod };
}
