// SPDX-License-Identifier: MIT

import test from 'node:test';
import assert from 'node:assert/strict';
import { canonical, parseJson, plan, sha256, validateCohort } from './contract.mjs';
import { clone, cohort, hash } from './test-fixtures.mjs';

test('plans both arms exactly once with reproducible order and no launch', () => {
  const value = cohort();
  const first = plan(value, hash('manifest'));
  assert.deepEqual(first, plan(value, hash('manifest')));
  assert.equal(first.scheduled.length, value.pairs.length * 2);
  assert.equal(first.launches_performed, 0);
  for (const pair of value.pairs) {
    assert.deepEqual(first.scheduled.filter(row => row.pair_id === pair.pair_id).map(row => row.slot), [0, 1]);
  }
});


for (const [name, mutate] of [
  ['floating model', c => { c.pins.model = 'jev-latest'; }],
  ['missing pin', c => { delete c.pins.protocol_sha256; }],
  ['unknown field', c => { c.untrusted = true; }],
  ['duplicate pair', c => { c.pairs.push(clone(c.pairs[0])); }],
  ['duplicate seed repetition', c => { const p = clone(c.pairs[0]); p.pair_id = 'other'; c.pairs.push(p); }],
  ['train-test seed leakage', c => { c.pairs[1].seed_sha256 = c.pairs[0].seed_sha256; c.pairs[1].repetition = 1; }],
  ['negative repetition', c => { c.pairs[0].repetition = -1; }],
  ['oversized repetition', c => { c.pairs[0].repetition = 100; }],
  ['invalid split', c => { c.pairs[0].split = 'test-ish'; }],
  ['empty cohort', c => { c.pairs = []; }],
  ['same policy pins', c => { c.pins.tactical_policy_sha256 = c.pins.baseline_policy_sha256; }],
]) {
  test(`cohort rejects ${name}`, () => {
    const value = cohort(); mutate(value);
    assert.throws(() => validateCohort(value));
  });
}

test('repetitions within one split remain valid', () => {
  const value = cohort();
  value.pairs.push({ ...value.pairs[1], pair_id: 'repeat', repetition: 1 });
  assert.equal(validateCohort(value).pairs.length, 3);
});

test('canonical equality ignores object insertion order, not array order', () => {
  assert.equal(canonical({ b: 1, a: [1, 2] }), canonical({ a: [1, 2], b: 1 }));
  assert.notEqual(canonical([1, 2]), canonical([2, 1]));
  assert.notEqual(sha256('{"x":1}'), sha256('{ "x": 1 }'));
});

for (const [name, bytes] of [
  ['plain duplicate keys', Buffer.from('{"a":1,"a":2}')],
  ['escaped duplicate keys', Buffer.from('{"a":1,"\\u0061":2}')],
  ['nested duplicate keys', Buffer.from('{"x":[{"a":1,"a":2}]}')],
  ['UTF-8 BOM', Buffer.from('\ufeff{"a":1}')],
  ['invalid UTF-8', Buffer.from([123, 34, 120, 34, 58, 34, 255, 34, 125])],
  ['unsafe integer', Buffer.from('{"generation":9007199254740993}')],
  ['overflow', Buffer.from('{"x":1e999}')],
  ['excessive nesting', Buffer.from('['.repeat(66) + '0' + ']'.repeat(66))],
  ['syntax error', Buffer.from('{not json}')],
]) {
  test(`strict JSON rejects ${name}`, () => assert.throws(() => parseJson(bytes)));
}

test('strict JSON preserves valid escapes, nested structures and repeated keys in separate objects', () => {
  const value = { 'a"b': '\\', rows: [{ a: 1 }, { a: 2 }], empty: {}, arr: [], yes: true, nil: null, n: -0.4 };
  assert.deepEqual(parseJson(Buffer.from(JSON.stringify(value))), value);
});
