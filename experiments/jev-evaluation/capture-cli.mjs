// SPDX-License-Identifier: MIT

import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { jsonFile, localRecord } from './io.mjs';
import { auditCaptures } from './capture-audit.mjs';

export async function main(args) {
  if (args.length !== 1) throw new Error('capture_arguments');
  const { value } = await jsonFile(args[0]);
  const report = await auditCaptures(value, descriptor => localRecord(args[0], descriptor));
  process.stdout.write(`${JSON.stringify(report)}\n`);
  return report.incomplete ? 3 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).then(code => { process.exitCode = code; }).catch(() => {
    // Do not echo filenames, parser excerpts, or arbitrary imported values.
    process.stderr.write('capture audit rejected invalid or unavailable evidence\n');
    process.exitCode = 2;
  });
}