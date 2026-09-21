// SPDX-License-Identifier: MIT

// One explicit Unix process group. No shell, no retry, no provider SDK and no game-control port.
import { spawn } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import { requireThat } from './contract.mjs';

const OUTPUT_LIMIT = 8192;
const STDERR_LIMIT = 65536;
// Bound on local pipe cleanup after the process group is signalled. The child's inherited pipes
// close only once its `close` event fires, which on a host under load can lag group termination by
// hundreds of milliseconds, so this bound is sized above that jitter rather than just above the
// median. It is therefore a host-load-dependent contract value: `child_closed: false` means closure
// was not confirmed within the bound, not that cleanup failed. No later arm launches either way.
const CLEANUP_GRACE_MS = 1000;

export function runBridge(executable, args, bytes, { cwd, env, timeoutMs, signal }) {
  requireThat(process.platform !== 'win32', 'runner_unix_only');
  requireThat(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'runner_arm_deadline');
  if (signal?.aborted) return Promise.resolve({ status: 'cancelled', stdout: Buffer.alloc(0),
    elapsed_ms: 0, process_started: false, child_closed: true });
  return new Promise(resolve => {
    const started = performance.now();
    let child, timer, cleanupTimer, terminal, done = false, spawned = false;
    let output = [], outputBytes = 0, stderrBytes = 0;
    const killGroup = () => {
      if (!child?.pid) return;
      try { process.kill(-child.pid, 'SIGKILL'); } catch (error) {
        if (error.code !== 'ESRCH') terminal = 'io_failed';
      }
    };
    const finish = (status, childClosed) => {
      if (done) return;
      done = true;
      clearTimeout(timer); clearTimeout(cleanupTimer);
      signal?.removeEventListener('abort', abort);
      child?.stdin?.destroy(); child?.stdout?.destroy(); child?.stderr?.destroy();
      const stdout = status === 'complete' ? Buffer.concat(output) : Buffer.alloc(0);
      for (const chunk of output) chunk.fill(0);
      output = [];
      resolve({ status, stdout, process_started: spawned, child_closed: childClosed,
        elapsed_ms: Math.ceil(performance.now() - started) });
    };
    const stop = status => {
      if (terminal || done) return;
      terminal = status; killGroup();
      cleanupTimer = setTimeout(() => finish(terminal, false), CLEANUP_GRACE_MS);
    };
    const abort = () => stop('cancelled');
    try {
      child = spawn(executable, args, { cwd, env, shell: false, detached: true,
        stdio: ['pipe', 'pipe', 'pipe'] });
    } catch { finish('spawn_failed', true); return; }
    child.once('spawn', () => { spawned = true; });
    child.once('error', () => stop('spawn_failed'));
    child.on('close', code => {
      // Close is distinct from exit: inherited pipes must not outlive the deadline.
      killGroup();
      finish(terminal ?? (code === 0 ? 'complete' : 'bridge_failed'), true);
    });
    child.stdout.on('data', chunk => {
      outputBytes += chunk.length;
      if (outputBytes > OUTPUT_LIMIT) { chunk.fill(0); stop('output_bound'); }
      else if (!terminal) output.push(Buffer.from(chunk));
    });
    child.stderr.on('data', chunk => {
      stderrBytes += chunk.length; chunk.fill(0);
      if (stderrBytes > STDERR_LIMIT) stop('output_bound');
    });
    child.stdin.on('error', () => { /* Exit/close and the deadline determine the terminal status. */ });
    child.stdout.on('error', () => stop('io_failed'));
    child.stderr.on('error', () => stop('io_failed'));
    timer = setTimeout(() => stop('timeout'), timeoutMs);
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    child.stdin.end(bytes);
  });
}
