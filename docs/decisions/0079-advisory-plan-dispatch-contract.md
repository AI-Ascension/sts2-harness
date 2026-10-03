# ADR 0079: An advisory plan is revalidated at every dispatch, and reuses the existing plan shape

Status: accepted for the episode policy path in `crates/harness`. It describes tests and a
contract-strengthening comment against code that already shipped, and introduces no new plan
shape. It does not authorize a provider, native-host, game or deployment lane, and it claims no run
has exercised a plan against a live provider or the real game.

## Context

[#319](https://github.com/AI-Ascension/sts2-harness/issues/319) asked for an ordered plan instead of a
single action, so that where the next action follows from the one just taken, the second call does not
re-establish context the provider already had. The issue was blocked on three prerequisites, all of
which are now closed: `sts2-harness#317` (bounding the wasted re-asks),
`sts2-game-mod#171` (the host discloses what a reward holds), and `sts2-harness#318`, whose
multi-question System One request is [ADR 0078](0078-multi-question-system-one-request.md).

The question this record answers is therefore not "should a plan exist" but "what does admitting a
plan cost in authority, and where is that cost enforced". The issue's own constraint is the
load-bearing part: a plan is a prediction about a state that has **not happened yet**, so every way it
can be wrong must end it rather than be coerced, substituted or retried.

Reading the code, the answer was that the contract was already written, dispatched and recorded — and
that none of it was tested. `action_plan.rs` carried the entire plan contract: bounded step count,
catalog membership at build time, permitted stage sequences, per-step expected state, and disposal on
divergence — with no test file beside it. Every property was asserted only by reading the source.

## Decision

### The existing `Decision::Plan` / `ActionPlan` contract is the whole feature; a second shape is not added

The work packages in #319 (T1 schema, T2 dispatch-time validation, T3 evidence) are already
implemented in `exo/decision.rs`, `episode/action_plan.rs`, `episode/policy_router.rs` and
`bin/runtime_support/runtime_v3_recording.rs`. Adding a parallel plan type would have produced two
answers to "what may be dispatched next", and the weaker one would have been the one the runner
consulted. So this record covers the tests that were missing, plus one real defect those tests
exposed.

### The tests isolate one guard each, because the guards mask each other

The first version of the suite passed every mutation. That was not a weakness in the mutations; it
was three tests that could not tell which guard had been removed, because a second guard fired first
and produced the same `None`. A suite in which every negative test passes under every mutation is
not evidence of anything.

Each test is therefore constructed so that the guard it names is the **only** thing that can end the
plan:

- the generation test holds the catalog full, the step still offered, and the hand emptied exactly as
  predicted, so only the generation counter can refuse;
- the catalog-revalidation test withdraws the current step while generation, HP, seed and hand all
  still match, so only re-derivation against the live catalog can refuse;
- the unsettled test asserts on the plan's own contents, not on the next dispatch returning `None`.

That last one is the defect. `ActionPlan::action_completed(false)` cleared the remaining steps, but
every caller that received an unsettled result also dropped the whole plan before asking again, so
the clear was unreachable: the next `next()` returned `None` at the `settled` check whether or not the
steps were still there. The guarantee the comment claimed was being enforced by the caller instead of
by the plan. The test now asserts the disposal directly, and the comment at the call site records
why the plan owns it rather than delegating to a caller that must remember.

### A plan that the host undercuts is recorded as a fresh decision, not a reused step

`DecisionRecorder` already marked each dispatch with `reused_model_execution`, comparing the
originating execution ID against the current one. What was untested was the case where the plan is
**dropped**: the dispatch that follows is the same action either way, so the row looks identical
whether the plan was right or the plan was abandoned and the provider was asked again. Those are
different claims about a run and must not record the same.

The new test drives the real `ExoDecisionSource` over a scripted transport: the plan's first step
settles, the host then undercuts it, and the second dispatch must come from a second round trip. It
asserts the fresh row carries `model_execution_id` 2 and `reused_model_execution: false`. It fails
exactly when later plan steps stop revalidating, and no other test in that module does.

## Consequences

- The plan contract is now covered by nine tests in `episode/action_plan_tests.rs` and one
  end-to-end recording test, and every guard is proven load-bearing: removing any one of the
  generation check, the state-compatibility check, the catalog revalidation, the unsettled disposal,
  the step bound, the duplicate-id refusal, the empty-plan refusal or the terminal-step ordering
  fails at least one test.
- Where the host undercuts a plan, the run costs the round trip it was trying to save and behaves
  exactly as it does today. That is the accepted trade: round trips are removed only where reality
  matched the prediction.
- `ActionPlan::action_completed` now relies on its own disposal rather than on every caller's
  diligence. No caller's behaviour changed, so this is contract-strengthening, not a behaviour change.
- No new plan shape, no wire-format change and no schema bump. A reader of an existing bridge record
  sees the same fields.
- These are local unit and integration tests over a scripted transport. No native host, live provider,
  hosted or production evidence is claimed here, and none was gathered.

### The 17 pre-existing runtime-binary failures are environmental and were not treated as ours

`cargo test -p sts2-harness --bin sts2-harness-runtime` reports 17 failures in
`branch_continuation_runtime`, `exact_restore` and the lifecycle admission tests. They are present
unchanged at the parent commit `9ccca364` and were re-run there to confirm it rather than assumed:
base and head fail on the identical 17 names, and head adds one passing test.

They are not environmental by assumption either. The failures reduce to `ExactArtifactStore`
returning `Missing` from the confined write path in
`execution/exact_checkpoint_io.rs`: the directory walk over the blob prefix succeeds, the leaf
`openat` returns the expected first-write `ENOENT`, and the `O_TMPFILE` temporary that follows is
what cannot be created on this container's filesystem. Nothing in this change touches that store,
that path jail or those tests. Fixing it is a separate piece of work and is deliberately not folded
into a change about plan dispatch validation, where it would be an unreviewable second subject.

Refs #319.
