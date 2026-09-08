# ADR 0016: Owned worker execution and command-loop separation

## Status

Implementation candidate under the watchdog assignment. Native server-loop wiring,
bounded execution-thread shutdown and independent review remain required. This
decision does not enable a listener, start a provider or claim live recovery.

## Decision

The worker command processor retains its capacity-one lane and durable-store
ownership. After the authenticated response-before-start boundary, it may issue
one non-cloneable `WorkerExecutionTask` for the exact retained running handoff.
Another claim, changed row or wrong lane is rejected. This is an in-process
ownership transfer, not a serialized credential or a new admission authority.

The task owns the immutable handoff and approved execution fingerprint, together
with the opaque shared store. The executable prepares immutable runtime settings
and launch options before execution; runtime-local `Rc` values are constructed
inside the executing thread. Neither a mutable command-processor borrow nor a
store lease crosses gameplay/provider execution.

A returned completion retains the task and a private processor-identity marker.
The original processor alone may consume it, and a callback success cannot free
the lane without the matching durable terminal receipt. Direct lane release and
store close are rejected while that task remains outstanding. An error, dropped
task, dropped unconsumed completion or unwind retains UNKNOWN and latches the
admission gate before attempting persistence. Failure to persist keeps admission
closed; it does not manufacture a terminal result or permit replacement work.

Stop and probe remain commands on the separate processor. Existing execution-time
control fences still apply at decision and dispatch admission. This transfer does
not cancel an already dispatched action, kill a provider or prove settlement.

## Compatibility and validation

This additive owner-local Rust API changes no wire schema, frozen artifact,
database migration or cross-repository authority. Synthetic tests cover one-use
claims, durable completion, rejected false success, cross-processor completion,
drop/unwind, busy-store quarantine and authenticated stop/probe while an owned,
joined execution thread waits at a bounded channel barrier.

The task API spawns no thread. Production integration must own and bound thread,
provider and MCP cleanup; it must not infer those guarantees from the synthetic
barrier test or the compile-time `Send` check.
