# ADR 0067: Schedule reproducible benchmark suites and report honest paired denominators

Status: accepted for the harness-owned, source-only slice of issue
[#125](https://github.com/AI-Ascension/sts2-harness/issues/125) — frozen suite declarations, a
retry-safe scheduler and a sanitized aggregate report. It does not launch a game, mutate a profile or
spend provider credit, and it makes no native-completeness claim: an authorized native multi-policy
demo stays an explicit external gate (harness [#126](https://github.com/AI-Ascension/sts2-harness/issues/126)).
It is ratified when the change carrying it merges.

## Context

Issue #125 requires benchmarking several model configurations on the same predeclared seed cases with
isolated trials, repeat counts and honest paired results. The audited prerequisites (#104, #118, #121,
#122, #123, #126) gate integrated acceptance, but the triage record explicitly allows contract design
and effect-free fixtures to proceed first. The harness already owns benchmark declarations
([ADR 0021](0021-benchmark-manifest-foundation.md)) and exact-state identities; what was missing was a
layer that turns a declaration into a stable trial plan, keeps attempt lineage across retry and
restart, and reports paired outcomes without inventing certainty.

Three failure modes had to be excluded by construction:

1. scoring a trial twice, or losing that it was retried or cancelled, when a crash lands between
   scheduling and settlement;
2. treating an unmeasured quantity as zero, or an infrastructure failure as a defeat, so a policy
   that failed to run looks like a policy that lost; and
3. blending trials whose initial native state never verified into a certified exact-start statistic.

## Decision

A new module `crates/harness/src/benchmark_manifest/suite/` owns that layer, split so each file stays
inside the production size budget:

- **Frozen declaration (`manifest.rs`).** `SuiteManifest` names the benchmark, the ordered
  `SeedCorpus`, the ordered `PolicyConfig` axis, the repetition count, the evaluator revision, the
  declared `SuiteBudgets` and the predeclared metric names. Corpus randomization, the native game
  seed of each case and the optional provider sampling seed are separate typed fields and are never
  substituted for one another. `validate` bounds every axis and `digest` gives the suite revision
  identity, so changing the corpus, policies, evaluator or budgets produces a new revision and
  preserves prior results.
- **Stable plan and scheduler (`plan.rs`, `scheduler.rs`).** `plan` derives one trial per suite
  revision, case, policy and repetition, in deterministic case-major order, each with its own
  `suite-trial:` context namespace. `SuiteScheduler` keeps attempt lineage: `start` records a retry,
  `settle` is idempotent for an identical outcome and refuses a conflicting one, `cancel` settles no
  result, and `resume` replays recorded outcomes over the frozen plan. The observed attempt count
  travels on the outcome, so restarts do not erase it.
- **Result taxonomy (`results.rs`).** `TrialStatus` separates completion from budget censoring,
  cancellation, infrastructure failure and unknown outcome; only `Completed` may carry a
  `TrialResult`. Every metric is a `Measurement<T>` that is either `Measured` or explicitly
  `Unavailable`, so an unmeasured cost is never zero. A `NativeWitness` parses the
  `asc-state:v1:sha256:` namespace, so an arbitrary string cannot claim a verified start.
- **Scheduling and reporting refusals (`error.rs`).** `ScheduleError` and `ReportError` carry the
  bounded rejection vocabulary; no supplied field value, seed or digest is reflected in a message.
- **Sanitized report (`report.rs`, `comparison.rs`, `witness.rs`).** `ensure_metric_coverage` proves
  each planned trial has exactly one valid outcome and every predeclared metric is known, then counts
  metric availability. `aggregate` produces per-(case, policy) cells with explicit planned, recorded,
  decided, censored, cancelled, infrastructure-failure, unknown and missing denominators.
  `compare_paired_policies` pairs only repetitions decided in both policies, reports the missing
  pairs and returns no delta when nothing paired. `audit_initial_witness` groups each case by its
  verified witness and rejects two different witnesses for one case; later divergence in actions, RNG
  consumption or outcome is expected and is not a defect, and an unverified trial is excluded rather
  than blended in.

The owner API covers validate (`SuiteManifest::validate`/`digest`), run (`plan` plus
`SuiteScheduler::start`/`settle`), status (`pending`, `phase_of`, `attempts`), resume
(`SuiteScheduler::resume`) and export (`aggregate` plus `SuiteReport::to_json_pretty`) without a
Studio UI, matching requirement 6 of the issue.

## Consequences

- The eight-trial 2x2x2 synthetic suite is a contract, not a convention: keys, namespaces and cells
  are derived from the manifest, so a fixture cannot silently plan seven or nine trials.
- A crash/retry/cancel cannot double-score a trial or erase an attempt, because settlement is
  idempotent-by-identity and conflicting, and the frozen corpus is retained by the scheduler rather
  than mutated by progress.
- Reports keep failures out of the win column and state their denominators; a reader can always tell
  a 0-of-0 rate from a 0-of-2 rate and an unavailable cost from a zero cost.
- Cost: two extra fields per cell (missing, unplanned rejection) and one more type per axis. The
  alternative — inferring trials from outcomes — was rejected because it cannot distinguish an
  unplanned key from a missing planned one.

## Validation

- `crates/harness/tests/benchmark_suite.rs` plans the 2x2x2 suite (eight distinct keys, eight distinct
  namespaces, every cell once), checks the complete result table and metric availability, and checks
  the manifest rejections and revision identity.
- `crates/harness/tests/benchmark_suite_schedule.rs` covers retry, idempotent replay, conflicting
  settlement, cancellation, unknown/malformed input, resume and the frozen corpus.
- `crates/harness/tests/benchmark_suite_report.rs` covers censored, infrastructure, cancelled and
  unknown denominators, paired comparisons with missing pairs, the no-pair delta, and report
  sanitization.
- `crates/harness/tests/benchmark_suite_witness.rs` covers one shared start group under divergent
  outcomes, a conflicting-witness defect, exclusion of unverified trials and digest-namespace
  parsing.
- Source-only: no native run, provider call or profile mutation is exercised; native exact-start
  certification remains unverified until #126 records its evidence.

## References

- Issue [#125](https://github.com/AI-Ascension/sts2-harness/issues/125):
  "Run reproducible multi-model benchmark suites over a frozen seed corpus".
- [ADR 0021](0021-benchmark-manifest-foundation.md): private benchmark declarations and effect-free
  receipt association reused here.
