// SPDX-License-Identifier: MIT

// This CLI deliberately has no run/resume mode. The existing hash-approved runner owns execution.
import { planPilot, reportPilot } from './pilot.mjs';
import { publicError } from './runner-io.mjs';

try {
  const [command, path, ...extra] = process.argv.slice(2);
  if (!path || extra.length || !['plan', 'report'].includes(command)) {
    process.stderr.write('Usage: pilot-cli.mjs plan|report PRIVATE_MANIFEST\n');
    process.exitCode = 2;
  } else {
    const value = await (command === 'plan' ? planPilot(path) : reportPilot(path));
    process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
    process.exitCode = value.incomplete ? 3 : 0;
  }
} catch (error) {
  process.stderr.write(`${publicError(error)}\n`);
  process.exitCode = 2;
}
