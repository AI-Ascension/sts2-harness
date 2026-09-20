# Frozen Jev pilot: preflight and diagnostics

## Design and evidence scope

Proposed additive experiment profile `jev-frozen-pilot-v1`, based on merged #377.
This adds read-only tooling around the existing [paired runner](RUNNER.md), not another
provider or game execution path. It does not change the tactical questions, weights,
thresholds, bridge arguments, Rust capture contract, or the normal runtime output.
The profile's CLI/report contract is proposed for explicit design review with this change.

The first pilot asks an operational question: can the existing tactical evaluator produce
usable, attributable decisions on the approved inputs, and what prevents it when it cannot?
Ten pairs do not establish higher win rate or a calibrated probability of action quality.
Agreement with the baseline is not an accuracy label, and disagreement is not an improvement.

## Frozen scope

Use the existing `ascension.jev-paired-runner.v1` manifest with exactly ten pairs,
`max_pairs: 10`, and `max_provider_attempts: 20`. The execution budget is at most
1,800,000 ms and the original-input byte budget is at most 1,310,720 bytes. The existing
per-arm ceiling remains 120,000 ms. These are upper bounds, not a recommendation to
spend all of them. The existing runner still owns reservations, cancellation and deadlines.

The planner verifies executable byte pins, private file permissions, input hashes,
whole-input bounds, and the unchanged runner's cross-split leakage checks. It refuses
non-ASCII action identifiers because the compiled bridge in #377 rejects them before
transport. It never removes an invalid pair and substitutes another.

Single-action cases and catalogs larger than 24 remain visible in the plan rather than
being silently discarded. Raw catalog size is not proof that tactical evaluation will apply:
other bridge admission and request-size conditions can still cause a fallback or refusal.

Repetitions are permitted but are counted separately from distinct input-file hashes and
declared clusters. The planner additionally counts semantically identical inputs excluding
execution ID, using evaluator-local canonicalization, not Rust's capture fingerprint.
Cluster IDs remain operator assertions. Related but different snapshots require correctly
assigned clusters; the tool cannot reconstruct their native ancestry.

Do not change inputs, model pins, gate, weights, cohort or budget after seeing held-out
results. A revised experiment needs new reviewed bytes and new approval, not reinterpretation
of the original cohort. No parameter fitting, selection of winning seeds, or policy promotion
is performed by these tools.

## Prepare, execute, report

Provide ten approved original bridge inputs and the reviewed manifest described in
[RUNNER.md](RUNNER.md). Keep them in native Unix storage: directories owned by the user
with mode `0700`; regular, single-link files with mode `0600`; no symlink components.
The compiled bridge and operator-owned transport must already be reviewed and installed.
This tool does not install software, acquire credentials, or turn an example manifest into
approval to transmit private state.

Read-only preflight:

```sh
node experiments/jev-evaluation/pilot-cli.mjs plan /private/manifest.json
```

Review the returned manifest hash and budget. A passing local preflight is not a provider
availability check, a token-cost quote, proof of an immutable server model, or an execution
authorization. No credential environment values are accessed by the pilot CLI.

Execution remains an explicit invocation of the existing runner, only after approval of
the exact manifest hash and both provider calls per pair:

```sh
node experiments/jev-evaluation/runner-cli.mjs run /private/manifest.json --approve APPROVED_SHA256
```

The runner rechecks the approved bytes and executable pins before work. Do not retry an
interrupted run with a new directory simply to hide a failure. Its reserved and unknown
attempts remain part of the experiment accounting. A timeout does not prove that remote
provider work stopped. No hard token or dollar budget is implemented by the runner.

Read-only diagnostic report:

```sh
node experiments/jev-evaluation/pilot-cli.mjs report /private/manifest.json
```

There is deliberately no `run` or `resume` mode in the pilot CLI. The reporter reads the
original manifest and its run directory, not the original observation files or executables.
It recomputes the exact frozen schedule, validates the existing per-arm journals, reads only
their hash-bound capture descriptors, and independently reruns the capture audit. Cached
`audit*.json` files and `pairs.json` cannot supply replacement claims. The saved final summary,
when present, must equal the recomputed execution/audit accounting.

## What the report measures

The report gives overall, calibration, and held-out diagnostic groups. It separates execution
failures, incomplete captures, model drift, forced actions, tactical fallbacks and refusals
from the denominator of independently comparable action pairs. Per-arm refusal rates expose
both denominators: scheduled arms and complete captures. Neither hides the other.

Tactical gate diagnostics reproduce the current declared gates: low evidence in any candidate,
low confidence in the leading candidate, insufficient safety, and an insufficient utility
margin. Reasons may overlap. They describe why the policy refused, not why a game would be
won or lost, and their counts must not be summed as mutually exclusive outcomes.

Token, transport-attempt, process-elapsed and capture-elapsed measurements retain known counts,
known sums and unknown counts, with descriptive means, medians and extrema. Their scope is the
scheduled arms; known zero non-starts remain zero and unknown values are not imputed. Paired
differences are `tactical - baseline` on matching, complete provider-work pairs, including
refusals. Drift, forced actions and fallback comparisons are excluded from that matched-work
denominator. Each metric reports additional pairs excluded for missing measurements.

The capture contract currently provides input tokens only. No output-token count, dollar
cost, end-to-end gameplay latency, reward, native outcome, confidence interval, or win-rate
estimate is fabricated. Ten repeated executions from one input are not ten independent game
situations. Reported hashes can be correlated and are not anonymization.

A report snapshot digest binds the validated plan, journals, referenced capture byte hashes
and final-summary hash. It is an integrity reference, not a signature, model/build attestation,
native witness, or defense against an owner who rewrites all the evidence consistently.
Reads are not an atomic filesystem snapshot or proof that the originating process has stopped.
An absent final summary leaves the report incomplete even if every arm journal exists.

CLI exit 0 means valid local preflight or a complete diagnostic cohort, depending on the command.
Exit 3 returns a valid but incomplete diagnostic report, including legitimate refusals and
fallbacks. Exit 2 means invalid arguments, inconsistent evidence or an I/O error. Errors expose
no private path, observation, action identifier, transport output or credential value.

## Implementation and validation

`pilot-profile.mjs` owns the narrow bounds and cohort counts; `pilot.mjs` owns private read-only
composition; `pilot-analysis.mjs` owns descriptive calculations. `runner-journal.mjs` exposes
its existing validated read through `readRun`; `inspectRun` keeps its previous output shape.
No third-party package, workflow, Rust source, provider credential flow, or runtime mutation
surface is added.

```sh
node --test experiments/jev-evaluation/*.test.mjs
cargo run --locked --package repo-policy -- --strict
```

The existing explicit compiled suite's ten-pair case additionally checks the pilot reader
against actual bridge captures, including the single-input repetition denominator:

```sh
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --package sts2-harness --bin sts2-jev-bridge
STS2_JEV_TEST_BRIDGE="$PWD/target/debug/sts2-jev-bridge" \
STS2_JEV_TEST_SOURCE_REVISION="$(git rev-parse HEAD)" \
  node --test experiments/jev-evaluation/compiled-bridge.integration.mjs
```

The new module tests use hand-authored synthetic journals and local files only. Passing them
does not establish a provider pilot, compiled bridge execution, a native game, or better play.
Consult the change's validation report for the commands actually executed in its environment.
