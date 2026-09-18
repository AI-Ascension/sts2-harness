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
{"decision":"reobserve",
 "rationale":"bridge-authored evidence: chose play:card-defend-1 at p=0.56, runner-up play:card-bash-1:enemy-0 at p=0.35, confidence 0.45"}
```

A second call on the same state returned the same ordering with confidence `0.45`, so the two runs
agree on shape and differ in the third decimal.

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
