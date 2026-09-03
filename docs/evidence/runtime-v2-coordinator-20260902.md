# Runtime-v2 coordinator component evidence

- Date: 2026-09-02
- Target: `sts2-harness`
- Evidence level: `confirmed` offline component evidence
- Runtime/live status: no game, host, profile, save, provider, gateway, or MCP process was used by
  this seam

## Scope

The pure `RuntimeV2Coordinator` seam was added under `crates/harness/src/runtime_v2/`. It provides:

- a hard maximum of four registered instance lanes;
- separate process-port, gateway-session, MCP-session, lease, run, episode, trajectory, request,
  operation, trace, and artifact identities;
- one serial active dispatch slot per instance;
- FIFO ordering within a lane and round-robin scheduling across idle lanes;
- bounded global and per-instance waiting queues;
- explicit rejection for unknown/mismatched lineage, duplicate in-flight operations, retained
  operation IDs, and overload;
- queued cancellation before dispatch;
- shutdown that cancels queued work but reports active operation IDs for downstream settlement or
  reconciliation; and
- sanitized global/per-instance counters with a bounded 256-operation tombstone window;
- explicit unknown-outcome, rejection, and cancellation counts covering admission/queue and
  terminal outcomes; and
- optional dispatcher-supplied service-time sample, total-millisecond, and maximum-millisecond
  counters at global and per-instance scope.

## Deterministic checks

The isolated worktree passed:

```text
cargo run --locked --offline --package repo-policy -- --strict
Policy check: 82 sized files, 0 warning(s), 0 error(s)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
11 harness library tests passed; all workspace target tests passed
git diff --check
```

The focused tests cover four-lane registration and namespace reuse, fair serial dispatch, global and
per-instance overload without forwarding, retained operation reuse rejection, queued cancellation,
active-work shutdown accounting, and per-instance unknown/service-time metric accounting.

## Limits

This evidence does not prove production supervisor behavior, gateway allocation or lease ownership,
MCP transport composition, distinct disposable profiles, process-port liveness, host crash recovery,
downstream credential rotation, or live gameplay settlement. Those claims remain `unverified` and
require the authorized controlled environment in the aggregate Runtime-v2 completion report.
