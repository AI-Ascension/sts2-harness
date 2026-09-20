// SPDX-License-Identifier: MIT

import { open, lstat, realpath } from 'node:fs/promises';
import { constants } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, sep } from 'node:path';
import {
  MAX_JSON_BYTES, MAX_PAIRS, MAX_RESULTS_BYTES, ValidationError, decodeUtf8, parseJson, requireThat, sha256,
} from './contract.mjs';

export async function readBounded(path, maximum = MAX_JSON_BYTES) {
  let handle;
  try {
    const stat = await lstat(path);
    requireThat(stat.isFile() && !stat.isSymbolicLink(), 'not_regular_file');
    // O_NOFOLLOW also closes the final-component symlink substitution window on Unix.
    handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    requireThat((await handle.stat()).isFile(), 'not_regular_file');
    const buffer = Buffer.alloc(maximum + 1);
    let size = 0;
    while (size < buffer.length) {
      const { bytesRead } = await handle.read(buffer, size, buffer.length - size, null);
      if (bytesRead === 0) break;
      size += bytesRead;
    }
    requireThat(size <= maximum, 'file_size_bound');
    return buffer.subarray(0, size);
  } catch (error) {
    if (error instanceof ValidationError) throw error;
    throw new ValidationError('file_unavailable');
  } finally {
    if (handle) await handle.close();
  }
}


export async function jsonFile(path, maximum = MAX_JSON_BYTES) {
  const bytes = await readBounded(path, maximum);
  return { value: parseJson(bytes), sha256: sha256(bytes) };
}

export async function resultsFile(path) {
  const bytes = await readBounded(path, MAX_RESULTS_BYTES);
  const text = decodeUtf8(bytes);
  requireThat(!text.startsWith('\ufeff'), 'byte_order_mark');
  const lines = text.split('\n');
  const rows = [];
  for (const line of lines) {
    if (line.trim().length === 0) continue;
    requireThat(Buffer.byteLength(line) <= 8192, 'result_line_bound');
    requireThat(rows.length < MAX_PAIRS * 2, 'result_count_bound');
    rows.push(parseJson(Buffer.from(line)));
  }
  return { rows, sha256: sha256(bytes) };
}

export async function localRecord(manifestPath, descriptor) {
  requireThat(typeof descriptor.path === 'string' && descriptor.path.length <= 240
    && /^[A-Za-z0-9_.\/-]+$/.test(descriptor.path)
    && !isAbsolute(descriptor.path)
    && descriptor.path.split('/').every(part => part !== '..' && part !== ''), 'record_path');
  const root = await realpath(dirname(resolve(manifestPath)));
  const candidate = resolve(root, descriptor.path);
  let actual;
  try {
    actual = await realpath(candidate);
  } catch {
    throw new ValidationError('file_unavailable');
  }
  const within = relative(root, actual);
  requireThat(within !== '..' && !within.startsWith(`..${sep}`) && !isAbsolute(within),
    'record_path_escape');
  // Records are operator-local evidence, not a sandbox against a hostile filesystem owner.
  const result = await jsonFile(candidate);
  requireThat(result.sha256 === descriptor.sha256, 'record_digest_mismatch');
  return result.value;
}
