# Offline Jev paired evaluation

Dependency-free experiment tooling for the opt-in tactical profile merged in PR #368,
reviewed at `1a075bcd43380dbe45a4bb33c2f822d6777c5a17`.
This is an **offline artifact reader and comparison planner**, not a game runner, provider
client, Rust runtime change, or evidence that tactical play is better.

## Run the verified local checks

Node.js 22.16.0 was used for the local validation. No package installation or credentials
are required. `package.json` declares the minimum runtime for this experiment.

```sh
cd experiments/jev-evaluation
node --test contract.test.mjs outcomes.test.mjs audit.test.mjs io.test.mjs
node demo.mjs
node cli.mjs plan ../../target/jev-evaluation-demo/cohort.json
node cli.mjs audit ../../target/jev-evaluation-demo/pairs.json
node cli.mjs compare ../../target/jev-evaluation-demo/cohort.json ../../target/jev-evaluation-demo/results.jsonl
```

The demo writes only hand-authored **synthetic** records under `target/jev-evaluation-demo`.
It refuses to overwrite an existing directory. An optional argument selects a new output
directory. Demo outcomes are arbitrary test inputs, not observations of Jev or a game.

The evaluator returns exit code 0 for a complete valid analysis, 3 for an incomplete
analysis that still emits a report, and 2 for a malformed manifest/result or an I/O failure.
Exit 0 is not a gameplay success, a statistical finding, or policy-promotion authorization.

## Two distinct experiments

### 1. Compare separately captured decisions

The bridge's `tactical.assessment.baseline_choice` is the action question answered inside
the *same expanded tactical request*. It is not an independent baseline invocation.
The tactical request also wraps the original state in a documented tactical context.
This reader tracks two separate quantities:

- Agreement between a separately recorded baseline decision and a tactical decision.
- Agreement between the tactical selection and its own within-request action choice.

Only action/action pairs contribute to the independent agreement denominator. Refusals,
missing records, mismatched inputs, and forced records remain visible as separate counts.
Action disagreement is not, by itself, evidence that either policy is better.

An operator prepares a `pairs.json` manifest using the demo as an exact schema example.
Each pair names two separately captured `--record` output files, distinct
`model_execution_id`-equivalent execution identifiers, the digest of the original bridge
input, the expected bridge-binary digest, and SHA-256 digests of the raw record-file bytes.
The `execution_id` field belongs only to this private import format; it does not replace
the runtime's `model_execution_id` namespace.

Record files must be relative to the manifest directory. The reader does not fetch files
from the network, execute a bridge, open an arbitrary absolute record path, or infer
missing executions. Existing reviewed capture/retention policy still applies to inputs.

```sh
node cli.mjs audit /approved/private/evidence/pairs.json > /approved/private/evidence/audit.json
```

The audit requires the requested and returned model IDs to equal the manifest's explicit
model pin. Common rolling names such as `jev-latest` are refused. The provider must still
supply an independently reviewed immutable model identity; rejecting common aliases does
not guarantee that an arbitrary provider name is immutable.

For applied tactical requests, normalization removes **only** the exact v1 wrapper defined
in `jev_tactical_request.rs`. Its context, limits, question policy and profile are checked;
all remaining shared request content and the action question must match the separate
baseline. A changed objective, generation, observation, model or candidate description
is not silently normalized away. Catalog folding may make some pairs incomparable; those
cases remain counted and should be reviewed rather than discarded from the experiment.

The record-shape checker validates row/catalog consistency, utility arithmetic, threshold
and profile identity, decision binding, model binding, and non-forceable tactical refusals.
It does **not** re-execute the complete Rust response validator or selector. The executed
bridge and its normal validation remain authoritative.

The `low_evidence_estimate` diagnostic means the model assigned low evidence sufficiency.
It does not prove that block, card effects, or any particular host field is absent.
Inspect the approved observation before attributing a refusal to a missing field.

### 2. Account for matched-seed episode outcomes

A `cohort.json` manifest freezes the planned pairs, calibration/held-out split, repetitions,
and independent pins for the model, source revision, bridge, game build, mod, protocol,
observation policy, budget, and both policies. The output of `plan` schedules both arms
with a deterministic hash-based AB/BA order. It does not launch or reserve anything.
The operator must freeze and retain the manifest *before* collecting results; a hash
proves byte identity, not when the document was written.

```sh
node cli.mjs plan /approved/private/evidence/cohort.json > /approved/private/evidence/plan.json
node cli.mjs compare /approved/private/evidence/cohort.json /approved/private/evidence/results.jsonl > /approved/private/evidence/report.json
```

Every result row must match the exact raw cohort-file digest, its pins, planned arm and
order slot. Duplicate arms, reused run/episode IDs, unplanned runs and seed leakage across
splits are refused. A repeat belongs in the preregistered cohort; it is not a replacement
for an unfavorable result. The imported run and episode namespaces stay distinct.

There is one terminal accounting row per planned arm. Supported outcomes are `victory`,
`defeat`, `timeout`, `infrastructure_failure`, `refusal_stall`, `interrupted` and
`not_started`. A missing row is `unreported`, **not** a defeat. Its start status is unknown.
`started` refers to the execution attempt, not proof that the game or provider was reached.

Victory/defeat rows require a terminal-witness digest. Its bytes, meaning, independence,
host-generation binding and settlement are **not** verified by this reader. The upstream
native evidence and receipt validators remain necessary. Process exit 0, an HTTP response,
a bridge action, or a model confidence score must never be mapped directly to victory.

Metrics are `provider_calls`, `input_tokens`, `latency_ms`, `cost_micro_usd`, and
`combat_hp_lost`. Use `null` when not measured. Calls and tokens are integer counts, cost
is an integer count of millionths of USD, and latency/HP loss may be nonnegative fractional
measurements. Supply costs from approved usage/billing evidence; this tool does not guess
current provider prices. Include all retries and failed calls in the operator accounting.

Reports retain known measurement counts and unknown counts. Partial observed sums are
separate from complete totals; missing costs or calls do not silently become zero.
`recorded_victory_fraction_of_scheduled` is a descriptive recorded-victory fraction, not an
imputation of missing runs as defeats. Operational paired differences include infrastructure,
timeout and refusal outcomes. Terminal-gameplay-only differences are explicitly labeled
potentially selection-biased when not all scheduled pairs reached victory or defeat.

Calibration and held-out results are separate. Repeated seeds are not independent samples;
the report includes an equal-seed-weight descriptive comparison as well as a pair-weighted
one. No confidence interval, significance test, win-probability claim or automatic promotion
is implemented. Preregister an appropriate seed-clustered statistical analysis before making
a gameplay-improvement claim.

## Data and safety boundary

The production reader imports only Node's filesystem/path/URL/crypto facilities. It has
no HTTP, shell, child-process, game, credential, or installation entry point. CLI process
tests launch only this local Node program with synthetic inputs.

Reports omit source state, prompts, provider text, action IDs/descriptions and local paths.
Use sanitized opaque IDs in manifests. Raw record-file bytes are digest-checked before
parsing; the JavaScript equality encoding is **not** a reconstruction of Rust's
`serde_json` request digest or a claim of RFC 8785 canonicalization.

JSON imports reject malformed UTF-8, byte-order marks, duplicate keys (including escaped
duplicates), unsafe integer values, excessive nesting and oversized files. Record reads
reject absolute/traversing paths and symlinks escaping the manifest root. They are not
an OS sandbox against a hostile owner who can concurrently rewrite the filesystem.

Limits: 4,096 pairs, fewer than 100 repetitions per seed, 1 MiB per JSON input/record,
16 MiB per result stream and 8 KiB per result line. The loader reads record pairs sequentially.

## Integration and validation boundaries

This separate JavaScript experiment does not introduce Node into the Rust harness runtime,
add a Cargo dependency, alter a game/protocol field, change an installed binary, or enable
the tactical profile by default. It has no automatic CI integration in this first package;
run its explicit test command in review in addition to the repository's existing gates.
Its private analysis manifests do not replace the Rust `benchmark_manifest` library,
receipt association or runtime admission. No live exporter or native runner is wired here.
Runtime admission still rejects `--record` on its decision-only stdout path; collect
standalone approved captures or add a separately reviewed redacted sidecar exporter rather
than enabling record mode in an admitted live invocation.

Before merging an integration, run the repository's strict policy command:

```sh
cargo run --locked --package repo-policy -- --strict
```

Follow the normal Rust gates if the integration also changes Rust source. A passing local
Node suite is not a passing Rust workspace, Windows runtime check, provider exchange, or
native seeded episode. The repository's policy and native-validation requirements remain
unchanged.

Source contract references, relative to the harness repository:

- [Bridge record assembly](../../crates/harness/src/bin/support/jev_record.rs).
- [Tactical request wrapper and fallbacks](../../crates/harness/src/bin/support/jev_tactical_request.rs).
- [Tactical selection and recorded evidence](../../crates/harness/src/bin/support/jev_tactical_selection.rs).
- [Testing and evidence requirements](../../docs/TESTING.md).

All included fixtures and the demo generator are hand-authored, synthetic, MIT-licensed
test material. They do not contain model output, host files, credentials, saves or datasets.
The intended next use is to audit approved real paired captures, then compare a frozen
held-out cohort through the existing reviewed seed/gateway/MCP pathway.
