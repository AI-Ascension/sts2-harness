# ADR 0016: Owned worker execution and command-loop separation

## Status

Implementation candidate under the watchdog assignment. The opt-in Linux runtime
now connects its native server loop to owned execution. Bounded execution-thread
shutdown, descendant containment and independent review remain required before
release. This decision does not claim live recovery or service validation.

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

The task API spawns no thread. The Linux executable owns one scoped execution
thread, joins its completed handle before releasing the lane, and keeps native
control requests separate from gameplay. SIGTERM/SIGINT close admission and retain
an active handoff as UNKNOWN before waiting for execution and closing the store.
An exit/unwind guard closes admission before the scope's fallback join. Completed
receipts are not overwritten by this uncertainty accounting. Execution failures
keep historical commands available and are retained for shutdown reporting.

The scope prevents detachment, but does not impose a hard join deadline. The Linux
shutdown owner now signals provider cancellation after fencing admission and
uncertainty. Exo pipe futures select cancellation against the existing exchange
deadline, then invoke bounded direct-child termination/reaping. Cancellation is
typed separately from timeout and is not retryable; durable recording retains
unknown consumption using the existing cancelled-provider classification.

The same signal reaches primary and recovery MCP exchanges. Cancellation drops
their pipe futures, poisons the session, and invokes bounded direct-child cleanup.
An already-cancelled execution cannot launch a replacement primary or recovery
MCP. A request written before cancellation remains uncertain; cancellation is
never converted to an authoritative rejection or a replacement dispatch.

Gateway connect/write/read now run in an owned, joined asynchronous exchange under
the same cancellation signal and absolute deadline. Cancelling a sent request
closes its socket without resending or inferring rejection. Explicit lease cleanup
uses its own bounded exchange after cancellation and does not reset the original
signal. Native worker tests retain a stalled allocation socket during SIGTERM and
acknowledge only the separate release; the handoff still remains UNKNOWN.

This signal does not prove remote inference cancellation, descendant containment,
or settlement of a game effect. Forced descendant cleanup and active-execution shutdown bounds need
additional implementation and fault evidence; neither the synthetic barrier test
nor the compile-time `Send` check proves those guarantees.
