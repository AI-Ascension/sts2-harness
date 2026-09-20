// SPDX-License-Identifier: MIT

import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { executeRun, planRun } from './paired-runner.mjs';
import { inspectRun } from './runner-journal.mjs';
import { publicError } from './runner-io.mjs';

export async function main(args = process.argv.slice(2)) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  process.once('SIGINT', abort); process.once('SIGTERM', abort);
  try {
    let report;
    if (args.length === 2 && args[0] === 'plan') report = await planRun(args[1]);
    else if (args.length === 2 && args[0] === 'inspect') report = await inspectRun(args[1]);
    else if (args.length === 4 && args[0] === 'run' && args[2] === '--approve') {
      report = await executeRun(args[1], args[3], { signal: controller.signal });
    } else {
      process.stderr.write('Usage: runner-cli.mjs plan MANIFEST | run MANIFEST --approve SHA256 | inspect DIRECTORY\n');
      return 2;
    }
    process.stdout.write(`${JSON.stringify(report)}\n`);
    return report.incomplete ? 3 : 0;
  } catch (error) { process.stderr.write(`${publicError(error)}\n`); return 2; }
  finally { process.removeListener('SIGINT', abort); process.removeListener('SIGTERM', abort); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  process.exitCode = await main();
}
