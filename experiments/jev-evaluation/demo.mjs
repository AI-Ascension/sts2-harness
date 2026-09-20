// SPDX-License-Identifier: MIT
// Generates hand-authored synthetic examples only. Never connects to a provider or game.

import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sha256 } from './contract.mjs';
import { main } from './cli.mjs';
import { baselineRecord, cohort, pairManifest, result, tacticalRecord } from './test-fixtures.mjs';

async function generate(directory) {
  await mkdir(dirname(directory), { recursive: true });
  // Refuse an existing output directory rather than overwrite any operator artifacts.
  await mkdir(directory);
  const write = (name, value) => writeFile(join(directory, name),
    `${JSON.stringify(value, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
  const c = cohort();
  const cohortText = `${JSON.stringify(c, null, 2)}\n`;
  await writeFile(join(directory, 'cohort.json'), cohortText, { flag: 'wx', mode: 0o600 });
  const manifestDigest = sha256(cohortText);
  const outcomes = [['defeat', 'victory'], ['timeout', 'defeat']];
  const rows = c.pairs.flatMap((pair, index) => ['baseline', 'tactical'].map((arm, armIndex) =>
    result(c, pair, arm, outcomes[index][armIndex], manifestDigest)));
  await writeFile(join(directory, 'results.jsonl'), `${rows.map(row => JSON.stringify(row)).join('\n')}\n`,
    { flag: 'wx', mode: 0o600 });
  const pairs = pairManifest();
  for (const [arm, record] of [['baseline', baselineRecord()], ['tactical', tacticalRecord()]]) {
    const text = `${JSON.stringify(record, null, 2)}\n`;
    await writeFile(join(directory, `${arm}.json`), text, { flag: 'wx', mode: 0o600 });
    pairs.pairs[0][arm].sha256 = sha256(text);
  }
  await write('pairs.json', pairs);
  await write('plan.json', await main(['plan', join(directory, 'cohort.json')]));
  await write('outcome-report.json', await main(['compare', join(directory, 'cohort.json'), join(directory, 'results.jsonl')]));
  await write('decision-audit.json', await main(['audit', join(directory, 'pairs.json')]));
}


try {
  if (process.argv.length > 3) throw new Error('arguments');
  const directory = process.argv[2] ? resolve(process.argv[2])
    : fileURLToPath(new URL('../../target/jev-evaluation-demo/', import.meta.url));
  await generate(directory);
  process.stdout.write('Synthetic demo generated. No provider or game was launched. No gameplay result is claimed.\n');
} catch {
  process.stderr.write('demo_failed: use a new writable output directory\n');
  process.exitCode = 2;
}
