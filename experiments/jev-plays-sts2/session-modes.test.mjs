// SPDX-License-Identifier: MIT
// Execute the real shell control flow with socket-free command doubles; never a game or provider.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, writeFile, rm, access } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const source = fileURLToPath(new URL('./jev-loop.sh', import.meta.url));
const base = fileURLToPath(new URL('../../target/jev-session-modes/', import.meta.url));
const options = { skip: process.platform === 'win32', timeout: 15000 };
const fake = `#!${process.execPath}
import fs from 'node:fs';
import path from 'node:path';
const name = path.basename(process.argv[1]), args = process.argv.slice(2);
const root = process.env.JEV_HOME;
const log = (event) => fs.appendFileSync(root+'/calls', event+'\\n');
if (name === 'id') console.log('0');
else if (name === 'chown' || name === 'loginctl') {}
else if (name === 'pgrep') process.exit(args.includes('steam') ? 0 : 1);
else if (name === 'pkill') log('pkill');
else if (name === 'ss') console.log('LISTEN 127.0.0.1:15626');
else if (name === 'sleep') await new Promise(r=>setTimeout(r,20));
else if (name === 'python3') {
  log('launch:'+ (args.includes('--persistent-session')?'stream':'benchmark'));
  const stop = args[args.indexOf('--stop-file')+1];
  const deadline = Date.now()+3500;
  while (!fs.existsSync(stop) && Date.now()<deadline) await new Promise(r=>setTimeout(r,20));
  log(fs.existsSync(stop)?'native-stop-requested':'native-natural-exit');
} else if (name === 'sts2-gateway-runtime') {
  console.log('serving '+process.env.STS2_INSTANCE_ID);
  process.on('SIGTERM',()=>{log('gateway-reaped');process.exit(0)});
  await new Promise(r=>setTimeout(r,6000));
} else if (name === 'sts2-harness-runtime') {
  const marker=process.env.JEV_WORK_ROOT+'/profile/progress';
  log(fs.existsSync(marker)?'harness-progress-retained':'harness-first');
  fs.writeFileSync(marker,'retained');
  process.exit(Number(process.env.TEST_HARNESS_EXIT));
} else throw new Error('unexpected command '+name);
`;

async function fixture(t, mode, exit = 0, episodes = 1) {
  await mkdir(base, { recursive: true });
  const root = await mkdtemp(join(base, 'case-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const bin = join(root, 'commands'), work = join(root, 'work'), bundle = join(root, 'bundle');
  for (const p of [bin, bundle, join(work, 'rest-preservation-ad927-linux/profile')])
    await mkdir(p, { recursive: true });
  await writeFile(join(bin, 'package.json'), '{"type":"module"}');
  for (const name of ['id', 'chown', 'loginctl', 'pgrep', 'pkill', 'ss', 'sleep', 'python3'])
    await writeFile(join(bin, name), fake, { mode: 0o700 });
  await writeFile(join(bin, 'sudo'), '#!/bin/bash\nshift 2\nexec "$@"\n', { mode: 0o700 });
  await writeFile(join(bin, 'timeout'), '#!/bin/bash\necho timeout >> "$JEV_HOME/calls"\nshift\nexec "$@"\n', { mode: 0o700 });
  for (const name of ['sts2-gateway-runtime', 'sts2-harness-runtime'])
    await writeFile(join(root, name), fake, { mode: 0o700 });
  await writeFile(join(root, 'package.json'), '{"type":"module"}');
  for (const name of ['sts2-mcp-server', 'sts2-jev-bridge', 'systemone_transport.py', 'key.txt'])
    await writeFile(join(root, name), 'fixture-only\n');
  for (const name of ['campaign', 'stream'])
    await writeFile(join(bundle, `launch-linux-native-rest-${name}.py`), 'fixture-only\n');
  const script = (await readFile(source, 'utf8')).replace('/run/lock/jev-session.lock', join(root, 'session.lock'));
  await writeFile(join(root, 'loop.sh'), script);
  const child = spawn('/bin/bash', [join(root, 'loop.sh'), mode], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, JEV_HOME: root,
      JEV_WORK_ROOT: work, JEV_BUNDLE: bundle, JEV_EPISODES: String(episodes),
      TEST_HARNESS_EXIT: String(exit) }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  t.after(() => { if (child.exitCode === null) child.kill('SIGTERM'); });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const done = new Promise(resolve => child.once('close', code => resolve(code)));
  return { root, done, output: () => output,
    calls: async () => (await readFile(join(root, 'calls'), 'utf8')).trim().split('\n') };
}

test('benchmark still applies timeout and stops the native process', options, async t => {
  const f = await fixture(t, 'benchmark');
  await f.done;
  const calls = await f.calls();
  assert.ok(calls.includes('timeout'), f.output());
  assert.ok(calls.includes('launch:benchmark'));
  assert.ok(calls.includes('native-stop-requested'));
});

for (const code of [0, 2, 124]) test(`stream exit ${code} keeps the game open without retry`, options, async t => {
  const f = await fixture(t, 'stream', code);
  await f.done;
  const calls = await f.calls();
  assert.equal(calls.filter(c => c.startsWith('harness-')).length, 1, f.output());
  assert.ok(calls.includes('launch:stream'));
  assert.ok(calls.includes('gateway-reaped'));
  assert.ok(calls.includes('native-natural-exit'));
  assert.ok(!calls.includes('timeout') && !calls.includes('pkill') && !calls.includes('native-stop-requested'));
});

test('explicit stream resume retains the same native process and profile', options, async t => {
  const f = await fixture(t, 'stream', 0, 2);
  let session;
  for (let i = 0; i < 150; i++) {
    try {
      session = (await readFile(join(f.root, 'stream-session.path'), 'utf8')).trim();
      await access(join(session, 'stream-status.txt'));
      break;
    } catch { await delay(10); }
  }
  assert.ok(session, f.output());
  await writeFile(join(session, 'resume'), '');
  await f.done;
  const calls = await f.calls();
  assert.equal(calls.filter(c => c.startsWith('launch:')).length, 1, f.output());
  assert.deepEqual(calls.filter(c => c.startsWith('harness-')), ['harness-first', 'harness-progress-retained']);
  assert.ok(!calls.includes('native-stop-requested'));
});
