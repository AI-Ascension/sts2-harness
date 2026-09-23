# ADR 0065: Atomic Budget Reservation and Cancel/Restart for Bounded Parallel Analysis

## Status

Accepted for the Harness dynamic-analysis scheduling boundary. This record does
not authorize a provider, native-host, game, deployment, or paid-call lane, and
it adds no live provider behaviour.

## Decision

The bounded analysis route (`execute_plan_bounded`, ADR 0049) gains a
budget-reserved route, `execute_plan_bounded_reserved`. Every branch is a
*possible provider write* — a paid inference the owner may issue — so the owner
reserves units for it **before** the branch can dispatch, and the aggregate
reservation is atomic across all branches. `ParallelBudget` is the narrow seam
the scheduler uses to reserve, report and restore a reservation; the scheduler
never invents or holds a provider. `BranchBudgetLedger` is the in-memory
implementation and mirrors the admission-before-reservation discipline of
`crates/harness/src/workflow/provider.rs`.

## Reservation semantics

- A reservation is keyed by `BranchBudgetKey` = plan digest + node identity, so
  the same branch keeps one reservation across a cancel and a restart even
  though the owner loop rebuilds its scheduling state from scratch.
- `reserve` is a single atomic check-then-reserve under one mutex: a repeated
  reservation for the same key and cost is idempotent, and the aggregate can
  never exceed the owner limit. A branch the ledger cannot reserve is recorded
  as `Failed(BudgetExhausted)` and is **not dispatched**.
- A cancelled in-flight reservation is retained as `ReservationState::Unknown`,
  never refunded, because the branch may already have reached the provider.
  `complete`/`fail`/`mark_unknown` move a reservation out of `Reserved` exactly
  once.
- On restart, a branch whose reservation is held, `Unknown` or `Completed` is a
  possible earlier provider write: it is neither re-reserved nor re-dispatched
  and is joined as `Unknown`. Only a `Failed` reservation with no realized units
  may be retried, and that retry re-reserves idempotently.

## Evidence and limits

`crates/harness/tests/workflow_dynamic_parallel/budget.rs` proves the aggregate
never oversubscribes under a barrier race, that a branch which cannot reserve
does not dispatch, that cancel retains the reservation and restart adds no
duplicate reservation or dispatch, that a definite failure retries once while a
possible write is never repeated, and a negative control that oversubscribes
with the aggregate check removed and passes with `BranchBudgetLedger`. This is
owner/synthetic evidence at the scheduler boundary: no provider, native-host,
game, gateway or browser execution is claimed. Cap enforcement remains ADR
0049's; branch-state reporting to Studio (sts2-harness#98 lane 98-C) remains a
separate decision in `ascension-workflow-studio`.
