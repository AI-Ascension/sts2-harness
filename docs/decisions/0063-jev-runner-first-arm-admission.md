# ADR 0063: Admit the first Jev runner arm structurally rather than by fixture timing

Status: accepted for the `experiments/jev-evaluation` paired-runner test contract. It records the
strategy the owner selected for the wall-clock-sensitive global-time-budget test in issue
[#388](https://github.com/AI-Ascension/sts2-harness/issues/388); the repair it describes is already
on `main` (#392 `eaf1eeeeef58`, #393 `56de55388ea9`). It does not authorize a provider, native-host
or game lane and changes no runner artifact. It is the campaign disposition owner's decision, and it
is ratified when the change carrying it merges.

## Context

`experiments/jev-evaluation/runner.test.mjs` asserted that a global time budget stops the **first**
arm and journals `timeout`. That outcome is reachable only if the arm is *spawned* before the budget
expires: the runner computes `deadline = performance.now() + total_timeout_ms` before the first arm is
prepared (`paired-runner.mjs:97`). The pre-#392 admission re-check after the reservation file and slot
directory were written journalled `cancelled` with `process_started: false` when the budget had
already been spent, and the test then failed its first assertion, `counts.timeout === 1`, with
`0 !== 1`. The test therefore asserted a contract ("a global time budget leaves later arms explicitly
unstarted") as a bet on the wall-clock cost of the runner's own bookkeeping inside a small budget. It
is `unix`-gated and its `execute` helper passes no `AbortSignal`, so in that test `cancelled` could
only come from the pre-spawn branch. The red was load-dependent and CI masked it with a retry
(`run_attempt: 1` failure, `run_attempt: 2` success, run `35541953006`).

The issue named three candidate directions: make the fixture deterministic so the first arm is
guaranteed to be spawned; give the budget headroom relative to the pre-spawn cost; or relax the
assertion to `timeout + cancelled === 1`. The third was already rejected in the issue, because
`runner.test.mjs:137` (`observed_provider_attempts_unknown_arms`) also differs between the two
branches (1 vs 0), so a disjunction would weaken two assertions rather than one.

## Decision

The first scheduled arm is admitted **whenever a run starts**, and an admitted arm always launches its
child with the smaller of `per_arm_timeout_ms` and the remaining execution budget, floored at 1 ms
(`paired-runner.mjs:44-55`, `:102-106`). The execution budget governs the admission of the arms after
the first and each admitted arm's child deadline; it is not a deadline on the filesystem work that
precedes a launch. Pre-spawn expiry is no longer reachable for the first arm, so the test no longer
bets on the wall-clock cost of the runner's own bookkeeping.

Neither fixture-only direction was taken. Structural admission makes the invariant hold at every
budget at or above the contract floor (`total_timeout_ms >= 25`, `runner-contract.mjs:67-68`) instead
of moving a threshold that would flake again on a slower or busier host. The
timeout/cancelled/not_started distinction is kept rather than collapsed:

- the renamed `a global time budget bounds the first arm and leaves later arms explicitly unstarted`
  asserts the first arm is `timeout` with `process_started: true` and `counts.cancelled === 0`, and
  the second arm is `not_started` with `process_started: false`;
- the `cancelled` accounting that used to be the defect is covered on its own terms by
  `an interruption after the reservation cancels an arm instead of timing it out`.

## Consequences

- `experiments/jev-evaluation/RUNNER.md` records the admission rule as a documented runner contract,
  and `CHANGELOG.md` records the repair.
- The global-budget test is no longer flaky in the failing direction by construction, and the
  pre-spawn branch that was the defect is a first-class tested path rather than a race outcome.
- A sibling load sensitivity remains **unfixed and undecided**: several children in the same suite are
  bounded by wall-clock and pipe-close timing, so the suite is CPU-contention sensitive (see the
  [no-retry matrix evidence](../evidence/jev-evaluation-noretry-matrix-20260923.md)). It is recorded
  rather than de-flaked: the trigger is not reproducible on demand, and a principled fix would have to
  decide a contract bound for several other tests, which is its own decision and not a local
  robustness edit. No second defect is claimed — an earlier 6-repetition observation of 4 failures did
  not replicate in a later matrix at comparable load, and the fresh 12-repetition CI-pinned matrix
  recorded beside this change saw one first-attempt failure at load 25.56, far below the load ≥ 81 the
  earlier 4-of-6 observation was taken at. The consistent reading is a contention-sensitive mechanism,
  not a defect in the files the earlier sample named.
- The umask-independent permission fixture (#393 `56de55388ea9`) is a sibling repair in the same
  suite: `runner-contract.test.mjs` now sets the unsafe parent's mode with `chmod` after `mkdir`,
  matching its already umask-independent siblings.

## Evidence

| Claim | Label | Source |
| --- | --- | --- |
| The pre-spawn branch was reachable at the contract floor | `confirmed` | 20-rep differential: base `d1fd07af` 14 `timeout` / 6 `cancelled`; with #392 20/20 `timeout` |
| The repair is on `main` | `confirmed` | #392 `eaf1eeeeef58`, #393 `56de55388ea9` |
| The suite is CPU-contention sensitive | `confirmed` mechanism, `unproven` rate | [no-retry matrix evidence](../evidence/jev-evaluation-noretry-matrix-20260923.md) |
| A second defect exists in the files the earlier sample named | `unsupported` | the 4-of-6 sample did not replicate |

Refs #388. Related: [ADR 0064](0064-jev-runner-teardown-cleanup-bound.md).
