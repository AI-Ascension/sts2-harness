// SPDX-License-Identifier: MIT

// Private local storage only. Trusted ancestors and a non-hostile filesystem owner are required.
import { constants } from 'node:fs';
import { createHash } from 'node:crypto';
import { lstat, mkdir, open, realpath } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { ValidationError, parseJson, requireThat, sha256 } from './contract.mjs';
import { MAX_INPUT, absolutePath, payload, schedule, validateInput, validateRunner } from './runner-contract.mjs';

export function unixOnly() {
  requireThat(process.platform !== 'win32' && typeof process.getuid === 'function', 'runner_unix_only');
}

export async function privateDirectory(path) {
  unixOnly(); absolutePath(path);
  const stat = await lstat(path);
  requireThat(stat.isDirectory() && !stat.isSymbolicLink() && (stat.mode & 0o7777) === 0o700
    && stat.uid === process.getuid() && await realpath(path) === path, 'runner_private_directory');
}

async function fileHandle(path, privateFile) {
  absolutePath(path);
  requireThat(await realpath(path) === path, 'runner_symlink');
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const stat = await handle.stat();
    requireThat(stat.isFile(), 'runner_regular_file');
    if (privateFile) {
      requireThat(stat.uid === process.getuid() && (stat.mode & 0o7777) === 0o600 && stat.nlink === 1,
        'runner_private_file');
    } else {
      requireThat([0, process.getuid()].includes(stat.uid) && (stat.mode & 0o7022) === 0
        && (stat.mode & 0o111) !== 0, 'runner_executable_mode');
    }
    return handle;
  } catch (error) { await handle.close(); throw error; }
}

export async function privateBytes(path, maximum) {
  const handle = await fileHandle(path, true);
  try {
    const buffer = Buffer.alloc(maximum + 1);
    let size = 0;
    while (size < buffer.length) {
      const { bytesRead } = await handle.read(buffer, size, buffer.length - size, null);
      if (bytesRead === 0) break;
      size += bytesRead;
    }
    requireThat(size <= maximum, 'runner_file_bound');
    return buffer.subarray(0, size);
  } finally { await handle.close(); }
}

export async function verifyExecutable(descriptor) {
  const handle = await fileHandle(descriptor.path, false);
  try {
    const hash = createHash('sha256'), buffer = Buffer.alloc(65536);
    let total = 0;
    for (;;) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
      if (bytesRead === 0) break;
      total += bytesRead;
      requireThat(total <= 128 * 1024 * 1024, 'runner_executable_bound');
      hash.update(buffer.subarray(0, bytesRead));
    }
    requireThat(hash.digest('hex') === descriptor.sha256, 'runner_executable_digest');
  } finally { await handle.close(); }
}

export async function createDirectory(path) {
  await privateDirectory(dirname(path));
  await mkdir(path, { mode: 0o700 }); // Exclusive; never adopt an existing run or reservation.
  await privateDirectory(path);
  await syncDirectory(dirname(path));
}

async function syncDirectory(path) {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try { await handle.sync(); } finally { await handle.close(); }
}

export async function createJson(path, value) {
  await privateDirectory(dirname(path));
  const bytes = Buffer.from(`${JSON.stringify(value)}\n`);
  requireThat(bytes.length <= 1024 * 1024, 'runner_journal_bound');
  const handle = await open(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL
    | constants.O_NOFOLLOW, 0o600);
  try { await handle.writeFile(bytes); await handle.sync(); } finally { await handle.close(); }
  await syncDirectory(dirname(path));
  return sha256(bytes);
}

export async function readOptionalJson(path, maximum = 16384) {
  try {
    const bytes = await privateBytes(path, maximum);
    return { value: parseJson(bytes), sha256: sha256(bytes) };
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
}

function environment(names, source) {
  const result = Object.create(null);
  for (const name of names) {
    requireThat(Object.hasOwn(source, name) && typeof source[name] === 'string'
      && source[name].length > 0 && Buffer.byteLength(source[name]) <= 16384
      && !source[name].includes('\0'), 'runner_environment_unavailable');
    result[name] = source[name];
  }
  return result;
}

// Read and pin all approved inputs before any child is launched. No raw input is copied to output.
export async function preflight(manifestPath) {
  unixOnly();
  const path = resolve(manifestPath), root = dirname(path);
  await privateDirectory(root);
  const bytes = await privateBytes(path, 1024 * 1024);
  const manifest = validateRunner(parseJson(bytes)), hash = sha256(bytes);
  await verifyExecutable(manifest.bridge); await verifyExecutable(manifest.transport);
  await privateDirectory(dirname(manifest.output_directory));
  let absent = false;
  try { await lstat(manifest.output_directory); } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    absent = true;
  }
  requireThat(absent, 'runner_output_exists');
  const entries = schedule(manifest, hash), inputs = [];
  const semantics = new Map();
  let total = 0;
  for (const [index, pair] of manifest.pairs.entries()) {
    const content = await privateBytes(join(root, pair.input_path), MAX_INPUT);
    requireThat(sha256(content) === pair.input_sha256, 'runner_input_digest');
    total += content.length;
    requireThat(total <= manifest.budgets.max_total_input_bytes, 'runner_total_input_bound');
    const input = validateInput(parseJson(content));
    const withoutExecution = { ...input }; delete withoutExecution.model_execution_id;
    // Evaluator-local identity detects repeated inputs crossing splits, not Rust fingerprints.
    const identity = sha256(JSON.stringify(sortJson(withoutExecution)));
    requireThat(!semantics.has(identity) || semantics.get(identity) === pair.split, 'runner_split_leakage');
    semantics.set(identity, pair.split);
    for (const entry of entries.slice(index * 2, index * 2 + 2)) payload(input, hash, entry.slot).fill(0);
    inputs.push(input); content.fill(0);
  }
  return { manifest, hash, entries, inputs };
}

function sortJson(value) {
  if (Array.isArray(value)) return value.map(sortJson);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortJson(value[key])]));
  }
  return value;
}

export const selectedEnvironment = (manifest, source = process.env) => environment(manifest.inherited_environment, source);

export function publicError(error) {
  // No OS error messages, paths, child output or arbitrary ValidationError text reach stdout/stderr.
  return error instanceof ValidationError ? 'runner_validation_failed' : 'runner_io_failed';
}
