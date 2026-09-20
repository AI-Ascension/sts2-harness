// SPDX-License-Identifier: MIT

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ValidationError } from './contract.mjs';
import { plan } from './contract.mjs';
import { jsonFile, localRecord, resultsFile } from './io.mjs';
import { compare } from './outcomes.mjs';
import { audit } from './audit.mjs';

export async function main(args) {
  const [command, manifestPath, resultPath] = args;
  if (!['plan', 'compare', 'audit'].includes(command) || !manifestPath
    || args.length !== (command === 'compare' ? 3 : 2)) {
    throw new ValidationError('usage: node cli.mjs plan COHORT | compare COHORT RESULTS_JSONL | audit PAIRS');
  }
  const manifest = await jsonFile(manifestPath);
  if (command === 'plan') return plan(manifest.value, manifest.sha256);
  if (command === 'audit') {
    const report = await audit(manifest.value, descriptor => localRecord(manifestPath, descriptor));
    return { ...report, manifest_sha256: manifest.sha256 };
  }
  const results = await resultsFile(resultPath);
  return {
    ...compare(manifest.value, manifest.sha256, results.rows), results_sha256: results.sha256,
  };
}


if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const report = await main(process.argv.slice(2));
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    if (report.analysis_status === 'incomplete') process.exitCode = 3;
  } catch (error) {
    // Never print user paths, prompts, provider text, stack traces, or credentials.
    process.stderr.write(`${error instanceof ValidationError ? error.code : 'evaluation_failed'}\n`);
    process.exitCode = 2;
  }
}
