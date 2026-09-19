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
| Response body digest | `sha256:a4f780156765337b367d72c4dca6bb3962d104f13cd7cad0e9940d74dcce107c` |
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
{"candidate_action_id":"play:card-defend-1",
 "candidate_confidence":44,
 "decision":"reobserve",
 "rationale":"bridge-authored evidence: chose play:card-defend-1 at p=0.55, runner-up play:card-bash-1:enemy-0 at p=0.37, confidence 0.44"}
```

Correction, 2026-09-19 (#305). The decision quoted here was the second call's, not this exchange's.
A second call on the same state returned the same ordering with confidence `0.45`, and its decision
was the one filed; the string carried `0.56`/`0.35`/`0.45` while the `provider_response` beside it
carries `0.55`/`0.37`/`0.44`, so no input to the bridge produced the string as filed. This entry now
carries the decision this bridge derives from the committed response, whole, and
`systemone_evidence_tests.rs` recomputes it from the artifact, so the two cannot drift apart again.
The second call has no committed artifact; nothing in this report depends on its numbers.

The `candidate_action_id` and `candidate_confidence` keys are part of that correction. `map_decision`
returns four fields for a re-observation rather than two, and the first filing carried only the
rationale and the decision name, which is a second, quieter instance of the same defect: a decision
written down by hand is a decision that can be written down incompletely. The filed object and the
bridge's output are now the same object, and the check compares them whole rather than field by
field, so a key added to the mapper cannot go missing from the record again. `sts2-jev-bridge
--record` prints the request, the response and the decision together for the next exchange, so an
operator can publish the bridge's own output instead of transcribing it.

## Digests and how to check them

Both digests cover the bytes of a compact JSON body — UTF-8, no whitespace — in the member order the
endpoint used. They differ in one respect: the request digest covers the body alone, with no trailing
newline, while the response digest covers the transported body including its single trailing newline.
The response body's member order is not the one the artifact beside this report preserves, so
compact-serializing the artifact does not reproduce the response digest even though it carries the
same values. The transported order, recovered against the published digest, is top level `model`,
`answers`, `usage`; `action` as `type`, `choice`, `confidence`, `probabilities`; and `probabilities`
as `play:card-bash-1:enemy-0`, `play:card-defend-1`, `combat.end-turn`,
`play:card-strike-1:enemy-0`, `play:card-strike-2:enemy-0`.

The transported response body is committed verbatim as
[`system-one-live-exchange-20260918.response-body.json`](system-one-live-exchange-20260918.response-body.json);
its SHA-256 is the response digest above. The request reproduces directly from the artifact's
`provider_request` under the compact rule above, with no trailing newline.
`systemone_evidence_tests.rs` asserts both, and asserts that the transported body carries the same
exchange as the artifact's `provider_response`.

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
