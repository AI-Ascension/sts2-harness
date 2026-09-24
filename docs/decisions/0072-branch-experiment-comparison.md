# ADR 0072: Plan, schedule and compare bounded same-start branch experiments

Status: accepted for the harness-owned, source-only slice of issue
[#119](https://github.com/AI-Ascension/sts2-harness/issues/119) — a versioned same-start declaration,
a per-child trial plan with an isolated context namespace, a same-start admission re-check, a
retry-safe recorded scheduler, an aligned comparison that separates policy divergence from restore
failure, and a sanitized report. It launches no game, restores no checkpoint and invokes no provider:
the restore, the child processes and the provider calls belong to the gateway and game-mod, and native
exact-host acceptance stays gated by sts2-game-mod#79. It is ratified when the change carrying it
merges.

## Context

Issue #119 requires several policies to be compared from one verified same-start checkpoint, and the
comparison to be reported honestly. [ADR 0069](0069-prefix-fork-admission.md) admits a single fork from
a verified seeded replay prefix, and
[ADR 0067](0067-reproducible-benchmark-suite-scheduling-and-reports.md) schedules independent
benchmark suites. Neither owns the experiment-level contract that fixes how one verified start is
shared by many children, how a child is scored exactly once across retries, or how two children are
compared without turning an identical endpoint into a false "no difference".

Three failure modes had to be excluded by construction:

1. two children of one experiment sharing context, provider state or identity;
2. a lost reply, a retry or a crash double-scoring a trial, or turning a censored or unknown trial
   into a defeat;
3. an endpoint-only comparison erasing an earlier divergence, or a failed restore being reported as
   a policy result.

## Decision

A new module `crates/harness/src/benchmark_manifest/branch_experiment/` owns that contract, split so
each file stays inside the production size budget, and is additive to the existing benchmark
machinery:

- **Versioned declaration (`declaration.rs`, `label.rs`).** `BranchExperimentManifest` freezes one
  verified `fork_point`, the `ForkStrategy`, the child policies, the stop conditions and the
  per-child/total `BranchBudgets`, all validated and bounded (`MAX_BRANCH_CHILDREN`,
  `MAX_BRANCH_LABEL_BYTES`, `MAX_BRANCH_CONCURRENCY`, `MAX_MANIFEST_BYTES`). `digest` is a canonical
  byte encoding hashed with the crate SHA-256, so identical declarations share one revision and no
  field is silently dropped. A child label carries a tighter bound (`MAX_CHILD_LABEL_BYTES`) than the
  other labels: since its derived key and context namespace add a 64-hex revision, a separator and
  `CONTEXT_NAMESPACE_PREFIX`, bounding the label at validation means a declaration that validates
  always derives a key and namespace the rest of the module accepts, instead of accepting a label it
  then refuses at every outcome check.
- **Stable plan (`plan.rs`).** `plan` derives one `PlannedBranchTrial` per child in declaration order,
  each with a stable `trial_key` derived from the revision and its own fresh
  `CONTEXT_NAMESPACE_PREFIX` namespace, so two children that share a policy still cannot share
  provider or context state.
- **Same-start admission (`admission.rs`).** `admit_start` re-checks that every trial restores the
  declared exact state at the declared checkpoint and that the assurance is a verified start; a
  prefix-only or capture-only start is refused or excluded from exact-restore statistics rather than
  counted as an exact restore.
- **Retry-safe scheduler (`scheduler.rs`).** `BranchExperimentScheduler` records attempt lineage: a
  retry increments attempts, a replayed settlement is an idempotent `Duplicate`, a conflicting
  settlement is refused, a settled or cancelled trial is never continued as a new start, and
  cancelling a trial settles no result.
- **Honest comparison (`comparison.rs`, `report.rs`).** `compare_branches` performs an aligned scan
  over logical actions: an intentionally different first action under a declared policy difference is
  `PolicyDivergence`, a failure to restore the shared state is `RestoreFailure` and never a policy
  result, and an equal endpoint does not erase an earlier divergence. Because a branch comparison has
  no authoritative side, the shorter peer trace is used as the baseline so an unequal-length pair is
  `IdenticalOverRecordedRange` with a non-zero `unobserved_records` on either argument order, rather
  than flipping between `MissingCapture` and `IdenticalOverRecordedRange` with the order.
  `aggregate` emits a sanitized `BranchExperimentReport` carrying a keyed handle and no exact digest.

## Consequences

- Two children of one experiment never share identity, context or provider state, because each trial
  owns a namespace derived from the immutable revision.
- A retried trial is scored at most once, and a censored, cancelled, infrastructure-failed or unknown
  trial stays out of a defeat tally and is never reported as a game result.
- Only a verified exact start is eligible for exact-restore statistics; a prefix-only start is
  reported honestly as ineligible.
- Cost: one new module and two new small vocabulary files (`plan.rs`, `report.rs`); the declaration
  digest is re-encoded without serde so the revision cannot drift with a serializer.

## Validation

- `crates/harness/tests/branch_experiment.rs` (declaration, plan, admission and scheduler) and
  `crates/harness/tests/branch_experiment_comparison.rs` (comparison and report), over the shared
  fixtures in `crates/harness/tests/support/branch_experiment.rs`, cover a stable plan with one
  namespace per child; the strategy/budget/duplicate/settings refusals; a max-length child label that
  validates, plans and settles while an over-long label is refused at `validate()`; same-start
  admission (including a mismatched and an unverified start); idempotent retry and
  conflicting-settlement refusal; a settled or cancelled trial that is never restarted; resume that
  replays attempt lineage once; cancellation and budget exhaustion staying out of a defeat tally; a
  restore failure that is not a policy result; a different first action classified as policy
  divergence at ordinal zero; an identical endpoint that does not erase an earlier divergence;
  unequal-length comparisons that are order-independent and surface their unobserved boundary count;
  incompatible and partial evidence labelled rather than guessed; a prefix-only start excluded from
  exact-restore statistics; and a public report that carries a keyed handle and no exact digest.
- Source-only: no native run, checkpoint restore or provider call is exercised; the exact-host
  same-start witness and the real child-process lane remain unverified until their gates record
  evidence.

## References

- Issue [#119](https://github.com/AI-Ascension/sts2-harness/issues/119): "Compare bounded same-start
  branch experiments".
- [ADR 0069](0069-prefix-fork-admission.md): the single-fork admission contract reused here.
- [ADR 0067](0067-reproducible-benchmark-suite-scheduling-and-reports.md): benchmark suite scheduling
  and reports.
- [ADR 0021](0021-benchmark-manifest-foundation.md): private benchmark declarations.
