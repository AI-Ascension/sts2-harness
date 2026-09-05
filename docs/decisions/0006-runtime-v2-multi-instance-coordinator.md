# ADR 0006: Runtime-v2 multi-instance coordinator seam

- Status: Accepted for component coordination; live supervisor and host isolation remain unverified
- Date: 2026-09-02

## Context

Runtime-v2 needs a harness-owned coordination boundary before it can drive more than one allocated
instance. The harness must preserve the gateway, MCP, lease, run, episode, trajectory, request,
operation, trace, and artifact namespaces while keeping game authority in the mod/host and lifecycle
authority in the gateway. A process driver or synthetic test must not be mistaken for proof of live
profile separation or host crash recovery.

## Decision

Add a pure `RuntimeV2Coordinator` seam under `crates/harness/src/runtime_v2/`. It accepts at most
four explicitly bound instance lanes, retains independent process-port and MCP-session references,
and uses one serial dispatch slot per instance. Waiting work is bounded both globally and per
instance. Admission returns explicit capacity or identity errors; it never forwards rejected work.
Ready lanes are scheduled round-robin, preserving FIFO order within each instance while allowing
other instances to make progress.

Shutdown closes admission, resolves queued items as explicit cancellation, and reports active
operation IDs for downstream settlement or reconciliation. It does not silently cancel active work,
own gateway leases, launch processes, contact MCP, or contact a game. Terminal operation retention
and replay remain downstream ledger responsibilities; the harness only keeps a bounded operation-ID
tombstone window to reject immediate reuse across instances.

Completion accepts an optional dispatcher-supplied service time in bounded milliseconds. The
coordinator records global and per-instance sample count, total, and maximum service time, plus
explicit unknown, rejection, and cancellation counts. Rejection and cancellation counts include
both admission/queue outcomes and terminal downstream outcomes. These are sanitized bookkeeping
fields only: the coordinator does not infer settlement, retry unknown work, or own a clock.

An `Unknown` outcome is not terminal: it retains the active operation and serial lane until an
explicit settled, rejected, or cancelled reconciliation arrives. Repeated unknown observations count
once per operation and do not count as completed work or service-time samples. Other instance lanes
remain schedulable, and shutdown continues to report uncertain active operations.

The coordinator rejects reused gateway-session, MCP-session, lease, process-port, run, episode,
trajectory, trace, or artifact identities across registered instances. Runtime-v2 operation IDs are
unique while queued or active. Sanitized snapshots expose queue/lifecycle counters without tokens,
prompts, host text, saves, or private paths.

## Consequences and evidence

The harness now has a deterministic component seam for four-instance admission, fairness, isolation,
backpressure, queued cancellation, active-work uncertainty, and shutdown accounting. Its Rust tests
are offline component evidence only. They do not prove a production supervisor, distinct disposable
profiles, real gateway/MCP process isolation, host crash recovery, downstream token rotation, or
live gameplay settlement. Those gates require the authorized controlled environment described by the
aggregate Runtime-v2 completion report.
