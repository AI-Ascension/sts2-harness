# System One live exchange, 2026-09-18

First recorded exchange between `sts2-jev-bridge` and the TypeSafe System One endpoint. This is an
operator-run, source-owner report. It is evidence about the provider lane only; it is not gameplay
evidence and no game was running.

## What was run

| Axis | Value |
| --- | --- |
| Bridge | `sts2-jev-bridge`, branch `feat/288-system-one-bridge-20260918`, debug build |
| Transport | reference Python transport from [`docs/JEV_MODEL_SELECTION.md`](../JEV_MODEL_SELECTION.md) |
| Endpoint | `https://api.typesafe.ai/v1/systemone` |
| Requested model | `jev-latest` |
| Model that answered | `jev-1.13.0` |
| Date | 2026-09-18 |
| Host | operator workstation, Linux |
| Request body digest | `sha256:057cc399be8d612dc8c7fe4bc11b2e20f8340785a8fee7df1ffdea281b59a2ce` |
| Response body digest | `sha256:b93444408488ec82e8f87f9e666e54728b4542f196043c0b64254ea8b3633bbf` |
| Digest canonicalization | sha256 over compact JSON: no insignificant whitespace, object keys sorted, array order preserved |
| Usage reported | 652 input tokens, 99 output tokens |

The observation was the synthetic combat fixture in
[`system-one-live-exchange-20260918.json`](system-one-live-exchange-20260918.json): 34 of 80 hit
points, 3 energy, two Strikes, a Defend and a Bash in hand, one enemy at 11 hit points intending 12
damage. Five legal actions were presented.

## What came back

```text
play:card-defend-1          0.55
play:card-bash-1:enemy-0    0.37
play:card-strike-1:enemy-0  0.04
play:card-strike-2:enemy-0  0.03
combat.end-turn             0.01
confidence                  0.44
```

The bridge emitted:

```json
{"decision":"reobserve",
 "candidate_action_id":"play:card-defend-1",
 "candidate_confidence":44,
 "rationale":"bridge-authored evidence: chose play:card-defend-1 at p=0.55, runner-up play:card-bash-1:enemy-0 at p=0.37, confidence 0.44"}
```

The decision is derived, not transcribed: `map_decision` composes it from the `choice`,
`confidence` and `probabilities` of the response above, so the same response always yields the same
four fields. The committed copy is now recomputed from the committed response by
`crates/harness/src/bin/support/system_one_evidence_tests.rs`, which fails if the two disagree.

## Corrections — 2026-09-19, [#305](https://github.com/AI-Ascension/sts2-harness/issues/305)

Three claims in the first version of this record did not survive review, and are withdrawn here
rather than restated. The withdrawal reasons are also machine-readable under `withdrawn_claims` in
the JSON.

- **The published response digest was not reproducible.** `a4f78015…` is not the digest of the
  committed `provider_response` under the canonicalization above, nor under 120 key orderings across
  five serializations. The received bytes it was taken over were not retained, so it can be neither
  confirmed nor recomputed. The digest now published, `b9344440…`, is the one any reader can
  recompute; the request digest was already reproducible and is unchanged.
- **The rationale did not belong to the response beside it.** It read `p=0.56`, runner-up `p=0.35`,
  confidence `0.45`; the committed response yields `p=0.55`, `p=0.37`, `0.44`, and no input to
  `map_decision` produces the recorded numbers. The stored decision is corrected to the mapper's
  output, including the `candidate_action_id` and `candidate_confidence` keys the bridge always
  emits for a re-observation and which the first version omitted.
- **The second-call claim had no artifact.** "A second call on the same state returned the same
  ordering with confidence `0.45`" is withdrawn: run 2 was never committed, so the sentence was
  unverifiable in either direction.

The root cause was transcription: the decision and the response were written down as two separate
fields by hand. `sts2-jev-bridge --record` now prints the request, the response and the decision as
one object, so an operator recording an exchange can publish the bridge's own output instead.

What the correction does **not** change: the exchange happened, the endpoint accepted the request,
the answer carried the documented shape, and the gate fired on a real answer — `0.44` is below the
`0.55` default, which is the conclusion the record always supported. The prose that drew on the
withdrawn numbers is corrected above; nothing else in this report depends on them.

## What this confirms

`confirmed` for this exchange, on this date, with these digests:

- The request this repository builds is accepted by the published endpoint, and the `criteria` keys
  round-trip: the returned `choice` is one of the action identifiers that were sent.
- The answer carries the documented shape — `choice`, a probability per option, and `confidence`.
- The bridge maps that answer to exactly one terminal decision, and the confidence gate fired: `0.44`
  is below the `0.55` default, so the bridge returned `reobserve` rather than acting on a
  distribution the model itself reports as spread. This is the gate doing the job it exists for, on
  a real answer, not a fixture.
- The transport contract holds end to end: one request body in on standard input, one response body
  out on standard output, credential read from the environment and never reaching the bridge process.

## What this does not confirm

- **Nothing about gameplay.** No game was running, the observation was synthetic, and no action was
  ever sent to a host. Model-played gameplay on this lane remains `unverified`.
- **Nothing about decision quality.** One state is not a measurement. Whether Defend over Bash was
  correct here is a judgement this report does not make; the model preferred blocking against 12
  incoming damage at 34 hit points and was not confident about it.
- **Nothing about the bridge's own TLS.** The exchange was made by the transport executable using its
  own TLS stack. The bridge still contains no TLS client, which is the subject of issue #299.
- **Nothing about a full run.** One call is not an episode. Rate behaviour, sustained cost, and
  latency under a real turn loop are unmeasured.

## Note for the confidence gate

The default gate of `0.55` is `proposed` and this is the first real data point against it. A
five-option combat choice with one plausible alternative produced `0.44`. If states like this one are
typical, a gate at `0.55` will return `reobserve` often, and the useful question becomes what the
runtime does with a re-observation that returns the same state. That is a policy question for the
episode runner, not for the bridge, and it should be answered with a distribution over many states
rather than this one.
