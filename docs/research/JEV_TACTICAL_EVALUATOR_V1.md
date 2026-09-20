# Jev Tactical Evaluator v1

Status: opt-in source candidate. Runtime, provider compatibility, calibration, and gameplay benefit
remain **unverified**. Synthetic tests are specifications, not recorded game outcomes.

## Owned boundary

The implementation belongs to the `sts2-jev-bridge` executable and its existing operator-owned
transport port. The library's pure coordination boundary, host-generated legal catalog, ordinary
harness/MCP/gateway/mod execution path, and post-action settlement remain unchanged. There is no new
network client, credential path, game rule, save access, simulator, or direct game operation.

`--tactical` is an explicit final suffix on either admitted execution argument shape:

```text
--model jev-1.13.0 --transport /absolute/provider-transport --tactical
--model jev-1.13.0 --transport /absolute/provider-transport --gate 20 --tactical
```

Omitting it retains the existing request, decision, and record shape. The flag is also available for
standalone `--record` and `--describe`; those two output modes remain prohibited as runtime execution
arguments. The bridge must be rebuilt and its new digest reviewed through the existing admission
procedure. Merely modifying the argument vector does not update an operator's installed binary.

Example operator configuration, after reviewing the new binary and its digest:

```text
STS2_PROVIDER_KIND=typesafe-jev
STS2_EXO_BRIDGE_ARGS_JSON=["--model","jev-1.13.0","--transport","/absolute/provider-transport","--gate","20","--tactical"]
```

The Windows transport path must be absolute for Windows. Keep the existing environment allowlist,
credential handling, and invocation authorization. No game was launched as part of this patch.

## One bounded evaluation

The candidate batch preserves the original action Choice for comparison, then asks six three-level
Score questions and one evidence-sufficiency Noul question per action. Axes are immediate benefit,
threat reduction, follow-up setup, resource preservation, strategy fit, and avoidance of severe
immediate downside. Each question includes its candidate description. Shared instructions and the
already-admitted observation/derived facts are supplied in state. Questions do not consume other
answers from the same request.

The tactical batch covers the full catalog or explicitly uses the unchanged legacy lane. It does
not discard candidates to fit its own budget. When the existing selector has grouped or reduced the
catalog, that legacy selection remains in charge rather than claiming that a subset is a complete
tactical comparison. Existing forced selections still avoid a provider call and are labeled as a
forced action or legacy forced group in tactical records.

Bounds: 24 candidates, at most 169 questions, 60 KiB for the serialized batch, and 24 KiB for serialized
state plus each individual question. The existing response and transport deadline bounds remain.
These are explicit application byte budgets, **not a tokenizer or a claim of exact token counts**.
Long descriptions or state can cause a fallback below the candidate-count limit. The original body
is retained and the reason recorded. Provider token limits continue to apply.

There is exactly one transport exchange for a non-forced decision. A malformed response causes a
failure, not a second call, guessed action, or partially populated evaluation. `--record` adds the
profile, applied/fallback status, actual returned model, full score matrix, weights, thresholds,
question-set digest, and request digest. It does not add credentials to the existing record.

## Validation and selection

Every requested answer must be present, with no extra answer IDs. The action Choice must have exactly
the requested probability keys, finite unit probabilities summing to one, and a choice at the maximum.
Each Score must have the expected three-level legend, normalized probabilities, a matching weighted
score, and finite unit confidence. Noul values must be finite and between zero and one.

Scores are normalized to [0,1] and combined with weights [3,3,2,2,2,4]. This is a transparent initial
heuristic, not a fitted value function. Model confidence is not a probability of completing a run.
Thresholds are proposed, not calibrated: all candidates require evidence sufficiency >= 0.8; the
winning action requires safety >= 0.5, margin >= 0.1, and minimum per-axis confidence >= the configured
gate. The evaluator can select a different action from the retained baseline Choice.

Insufficient evidence or a close/uncertain tradeoff returns `reobserve` **without** a candidate action
ID. Consequently the runner must not force that underinformed candidate after its abstention bound.
The existing episode re-observation limit remains authoritative. The patch does not add an automatic
stronger-model escalation or retry an unchanged request inside the bridge. A refusal can terminate
an episode when the runner's bound is reached; include such terminations in experimental results.

## Information boundary and known limitations

The existing `DerivedExactFacts` are retained. This change does **not** add authoritative block,
card-effect, target-modifier, or hidden-state fields to the protocol. Missing facts remain unknown;
Jev's sufficiency estimate is itself a model judgment, not proof that the observation is complete.
Neither the scoring rubric nor its safety axis proves that an action is safe or optimal.

Floating model aliases are convenient for operation, but comparisons should pin and record the actual
returned model. Three-level Score probability and legend validation follows the published API;
provider rounding and response compatibility must be verified before deployment.

## Validation to run after applying

```sh
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --package sts2-harness --bin sts2-jev-bridge --locked
cargo test --workspace --all-targets --all-features --locked
cargo run --locked --package repo-policy -- --strict
```

Use the repository's pinned toolchain. Existing CI includes the Windows bridge build; no workflow,
lockfile, branch protection, admission digest, or gate has been weakened by this patch.

The new synthetic cases cover batch construction, counter-baseline ranking, complete answer sets,
probability/legend/score consistency, confidence, missing evidence, candidate bounds, whole-request
bounds, catalog validation, one-call behavior, and default-path compatibility. The fixtures are
original test data, MIT-licensed with the repository, and contain no game assets or actual provider
responses. They do not establish semantic accuracy, live latency, win rate, or target-build coverage.

## Next experiment

First measure missing-evidence refusals and profile fallback frequency. Validate an admitted
player-visible effect/block projection for the exact host version if those are the dominant gaps.
Then freeze the model, question set, weights, game build, and budget and compare baseline against the
opt-in evaluator on held-out matched seed cohorts. Include every started run, separating defeat,
refusal, infrastructure failure, and timeout. Report completion rate, health loss, costs, latency,
provider calls, and uncertainty across whole runs. Keep shared seed/trajectory ancestry out of both
training and evaluation sets. Do not fit the selector or add search before this comparison.

Primary API reference: https://docs.typesafe.ai/api (retrieved September 20, 2026).
