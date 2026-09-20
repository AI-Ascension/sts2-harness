// SPDX-License-Identifier: MIT

// Original MIT test fixture: synthetic answers only, no socket or provider implementation.
// Copied to a private executable by the compiled-bridge tests, never used by the runtime.
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const FIXTURE_MODES = Object.freeze(['success', 'missing_usage', 'low_evidence',
  'low_confidence', 'model_drift', 'missing_answer', 'malformed', 'transport_failure']);
const hash = value => createHash('sha256').update(value).digest('hex');
const encoded = value => Buffer.from(JSON.stringify(value));
const byteOrder = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));

export function syntheticReply(body, mode = 'success') {
  if (!FIXTURE_MODES.includes(mode) || typeof body?.model !== 'string'
    || body?.questions?.action?.type !== 'choice') throw new Error('invalid synthetic request');
  const ids = Object.keys(body.questions.action.criteria).sort(byteOrder);
  if (ids.length === 0) throw new Error('empty synthetic catalog');
  const answers = Object.create(null), confidence = mode === 'low_confidence' ? 0.1 : 0.9;
  for (const [key, question] of Object.entries(body.questions)) {
    if (question.type === 'choice') {
      answers[key] = { type: 'choice', choice: ids[0], confidence,
        probabilities: Object.fromEntries(ids.map(id => [id, id === ids[0] ? 1 : 0])) };
    } else if (question.type === 'score') {
      if (!Array.isArray(question.criteria) || question.criteria.length !== 3) {
        throw new Error('invalid synthetic score rubric');
      }
      const good = question.instructions?.candidate_id === ids.at(-1);
      answers[key] = { type: 'score', score: good ? 1.8 : 0.2, confidence,
        legend: Object.fromEntries(question.criteria.map((label, index) => [String(index), label])),
        probabilities: { 0: good ? 0.1 : 0.9, 1: 0, 2: good ? 0.9 : 0.1 } };
    } else if (question.type === 'noul') {
      answers[key] = { type: 'noul', noul: mode === 'low_evidence' ? 0.05 : 0.95 };
    } else throw new Error('unexpected synthetic question type');
  }
  if (mode === 'missing_answer') {
    const key = Object.keys(answers).find(name => name !== 'action');
    if (key !== undefined) delete answers[key];
  }
  const reply = { model: mode === 'model_drift' ? 'jev-1.13.1' : body.model, answers };
  if (mode !== 'missing_usage') reply.usage = { input_tokens: 100, output_tokens: 100 };
  return reply;
}

async function boundedInput() {
  const chunks = [];
  let count = 0;
  for await (const chunk of process.stdin) {
    count += chunk.length;
    if (count > 128 * 1024) throw new Error('synthetic input bound');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function run() {
  const mode = process.env.JEV_RUNNER_FIXTURE_CASE, proof = process.env.JEV_RUNNER_FIXTURE_PROOF;
  // An explicit literal placeholder prevents accidental use of a real credential in this fixture.
  if (!FIXTURE_MODES.includes(mode) || !proof
    || process.env.TYPESAFE_API_KEY !== 'synthetic-secret') throw new Error('fixture not admitted');
  const bytes = await boundedInput(), body = JSON.parse(bytes);
  const reply = syntheticReply(body, mode);
  // Private test receipts contain hashes/counts, never the request, answers or credential value.
  await writeFile(join(proof, `${process.pid}.json`), `${JSON.stringify({
    request_sha256: hash(bytes),
    state_sha256: hash(encoded(body.state?.observation_and_derived_facts ?? body.state)),
    action_question_sha256: hash(encoded(body.questions.action)),
    question_count: Object.keys(body.questions).length,
    environment_names: Object.keys(process.env).sort(), synthetic_credential: true,
  })}\n`, { mode: 0o600, flag: 'wx' });
  if (mode === 'transport_failure') {
    process.stderr.write('SYNTHETIC_PRIVATE_TRANSPORT_ERROR\n');
    process.exitCode = 7;
    return;
  }
  process.stdout.write(mode === 'malformed' ? '{malformed' : `${JSON.stringify(reply)}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  run().catch(() => {
    process.stderr.write('compiled bridge fixture failed\n');
    process.exitCode = 2;
  });
}
