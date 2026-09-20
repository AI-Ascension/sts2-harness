// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { syntheticReply, FIXTURE_MODES } from './compiled-bridge-transport.mjs';

function request() {
  return { model: 'jev-1.13.0', state: 'synthetic private input', questions: {
    action: { type: 'choice', criteria: { first: 'Synthetic first', second: 'Synthetic second' } },
    score_first: { type: 'score', instructions: { candidate_id: 'first' }, criteria: ['bad', 'mixed', 'good'] },
    score_second: { type: 'score', instructions: { candidate_id: 'second' }, criteria: ['bad', 'mixed', 'good'] },
    evidence: { type: 'noul', criteria: 'Sufficient evidence' },
  } };
}

test('synthetic fixture deliberately disagrees between Choice and tactical scores', () => {
  const body = request(), before = structuredClone(body), reply = syntheticReply(body);
  assert.equal(reply.answers.action.choice, 'first');
  assert.equal(reply.answers.score_first.score, 0.2);
  assert.equal(reply.answers.score_second.score, 1.8);
  assert.deepEqual(reply.answers.score_second.legend, { 0: 'bad', 1: 'mixed', 2: 'good' });
  assert.equal(reply.answers.evidence.noul, 0.95);
  assert.deepEqual(reply.usage, { input_tokens: 100, output_tokens: 100 });
  assert.deepEqual(body, before);
});

test('synthetic score distributions reproduce their reported expectations', () => {
  for (const answer of Object.values(syntheticReply(request()).answers)) {
    if (!answer.probabilities) continue;
    assert.equal(Object.values(answer.probabilities).reduce((sum, value) => sum + value, 0), 1);
    if (answer.type === 'score') assert.equal(answer.score, Object.entries(answer.probabilities)
      .reduce((sum, [level, value]) => sum + Number(level) * value, 0));
  }
});

test('missing usage remains absent rather than being reported as zero', () => {
  assert.equal(Object.hasOwn(syntheticReply(request(), 'missing_usage'), 'usage'), false);
});

test('low-evidence and confidence fixtures change only the intended measurements', () => {
  assert.equal(syntheticReply(request(), 'low_evidence').answers.evidence.noul, 0.05);
  assert.equal(syntheticReply(request(), 'low_confidence').answers.action.confidence, 0.1);
  assert.equal(syntheticReply(request(), 'low_confidence').answers.score_first.confidence, 0.1);
});

test('missing tactical answer preserves an independent baseline reply', () => {
  const body = request();
  assert.equal(Object.keys(syntheticReply(body, 'missing_answer').answers).length, 3);
  body.questions = { action: body.questions.action };
  assert.deepEqual(syntheticReply(body, 'missing_answer'), syntheticReply(body));
});

test('model drift is explicit in the fixture response', () => {
  assert.equal(syntheticReply(request(), 'model_drift').model, 'jev-1.13.1');
});

test('fixture candidate order follows UTF-8 rather than JavaScript UTF-16 ordering', () => {
  const body = request();
  body.questions.action.criteria = { 'action-😀': 'astral', 'action-\uE000': 'BMP' };
  assert.equal(syntheticReply(body).answers.action.choice, 'action-\uE000');
});

for (const [name, change, mode] of [
  ['unknown mode', () => {}, 'typo'],
  ['missing action question', body => { delete body.questions.action; }, 'success'],
  ['empty catalog', body => { body.questions.action.criteria = {}; }, 'success'],
  ['unsupported question type', body => { body.questions.evidence.type = 'text'; }, 'success'],
  ['wrong score rubric', body => { body.questions.score_first.criteria = []; }, 'success'],
]) {
  test(`fixture rejects ${name}`, () => {
    const body = request(); change(body);
    assert.throws(() => syntheticReply(body, mode));
  });
}

async function scratch(t) {
  const base = fileURLToPath(new URL('../../target/jev-compiled-transport-tests/', import.meta.url));
  await mkdir(base, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(base, 'case-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

function invoke(root, mode, input = JSON.stringify(request()), extra = {}) {
  return spawnSync(process.execPath, [fileURLToPath(new URL('./compiled-bridge-transport.mjs', import.meta.url))], {
    input, encoding: 'utf8', timeout: 5000, maxBuffer: 256 * 1024,
    env: { JEV_RUNNER_FIXTURE_CASE: mode, JEV_RUNNER_FIXTURE_PROOF: root,
      TYPESAFE_API_KEY: 'synthetic-secret', ...extra },
  });
}

for (const mode of FIXTURE_MODES) {
  test(`fixture process emits one bounded receipt for ${mode}`, async t => {
    const root = await scratch(t), result = invoke(root, mode);
    assert.equal(result.error, undefined);
    assert.equal(result.status, mode === 'transport_failure' ? 7 : 0);
    const names = await readdir(root);
    assert.equal(names.length, 1);
    const text = await readFile(join(root, names[0]), 'utf8'), proof = JSON.parse(text);
    assert.equal(proof.question_count, 4);
    assert.match(proof.request_sha256, /^[0-9a-f]{64}$/);
    for (const forbidden of ['synthetic-secret', 'synthetic private input', 'Synthetic first', root]) {
      assert.equal(text.includes(forbidden), false);
    }
    if (mode === 'malformed') assert.throws(() => JSON.parse(result.stdout));
    else if (mode === 'transport_failure') assert.equal(result.stdout, '');
    else assert.equal(result.stdout, `${JSON.stringify(syntheticReply(request(), mode))}\n`);
  });
}

test('fixture refuses a non-placeholder credential before reading or recording input', async t => {
  const root = await scratch(t), result = invoke(root, 'success', JSON.stringify(request()),
    { TYPESAFE_API_KEY: 'not-the-synthetic-placeholder' });
  assert.equal(result.status, 2); assert.equal(result.stdout, '');
  assert.equal(result.stderr.includes('not-the-synthetic-placeholder'), false);
  assert.deepEqual(await readdir(root), []);
});

test('fixture refuses oversized input without a receipt or provider-like output', async t => {
  const root = await scratch(t), result = invoke(root, 'success', 'x'.repeat(128 * 1024 + 1));
  assert.equal(result.status, 2); assert.equal(result.stdout, '');
  assert.deepEqual(await readdir(root), []);
});
