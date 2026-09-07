# Runtime-v3 completed-resume evidence

- Date: 2026-09-07
- Target: `sts2-harness`
- Evidence level: `confirmed` offline/process-boundary evidence
- Runtime/live status: no game, host, provider, gateway, or MCP service was used

## Contract

`--resume` now admits a durable `Completed` episode and returns its stored
`CompletionRecord` directly. The fast path runs before construction of the gateway/MCP runtime
port, MCP child, Exo provider, or provider session. The output contains only the durable lineage,
status, terminal reference, checkpoint sequence, and result digest; it does not synthesize a new
observation, action, model decision, or report.

Resume admission requires seed (or visible-seed), build, and state fingerprint evidence. Missing
evidence fails closed. The MCP executable is included in the configuration fingerprint using a
bounded digest of a resolved regular, non-symlink file (absolute, relative, and PATH forms are
resolved by the runtime). This digest is identity evidence, not a TOCTOU execution barrier.
ReconstructionRequired and InterruptedUnknown remain denied until a separately approved
reconstruction path exists.

## Process oracle

The Linux-only supervisor places each child in its own process group and signals that exact group
through the safe `rustix` API while the leader remains waitable. `waitid(EXITED|NOHANG|NOWAIT)`
observes leader exit without releasing the PID, and the direct child is killed and reaped after
group signaling even when group cleanup reports an error. Both output pipes are nonblocking and
drained by one bounded `poll`/read loop; no reader threads are detached. Each drain call caps read
attempts and bytes consumed, including repeated `Interrupted` results and streams that never reach
EOF or `WouldBlock`, so the outer cleanup deadline always regains control. Focused regressions cover
a parent that exits while a descendant retains a pipe, a direct-child timeout, output overflow, and
adversarial continuous/interrupted readers; each completes under a short wall-clock bound.
Descendants that escape the owned process group are outside this containment claim, and other Unix
targets do not claim native coverage.

`crates/harness/tests/completed_resume_process.rs` seeds a SQLite episode with a durable checkpoint
and completion, then starts `sts2-harness-runtime --resume` under a bounded child supervisor with
explicitly cleared environment and concurrently drained, capped stdout/stderr. A real loopback
gateway listener counts connection attempts while executable probe files count MCP/provider
invocations. It verifies:

- the child returns the exact stored completion fields as one JSON line;
- missing seed evidence is rejected before external construction;
- `InterruptedUnknown` is rejected with the reconstruction-required error; and
- executable probe files used for MCP/provider boundaries are not invoked in any of those paths.

## Deterministic checks

The source-equivalent Linux checkout passed the focused gates below after resolving the pending
exact `rustix` dev-dependency lock entry (the root branch owns that lockfile update):

```text
cargo run --locked --package repo-policy -- --strict
Policy check: 391 sized files, 0 warning(s), 0 error(s)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --offline --locked --package sts2-harness --test completed_resume_process -- --nocapture
7 passed; repeated three times with 7 passed each
```

Before this supervisor replacement, the full workspace command was run twice on the preceding
candidate branch:

```text
cargo test --workspace --all-targets --all-features --locked
77 runtime tests passed; 1 failed
runtime_support::mcp_process::tests::full_duplex_does_not_deadlock_on_pipe_capacity
failure: MCP shutdown timed out
```

The exact failing test was rerun separately and failed with the same `MCP shutdown timed out`
result. The full workspace gate was therefore failed on that preceding candidate and was not
silently rewritten here; the H20 source-equivalent full workspace gate remains unverified pending
the root-owned lockfile update. The focused completed-resume evidence is 7/7 across three repeated
runs.

## Limits

This evidence does not prove Ready episodes with pending operations can be resumed, canonical
sideband recovery, host or game settlement, provider accounting across a crash, gateway lease
ownership, or live gameplay. Those remain separate recovery work and require the corresponding
authorized runtime and host evidence.
