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

The isolated worktree passed:

```text
cargo run --locked --package repo-policy -- --strict
Policy check: 391 sized files, 0 warning(s), 0 error(s)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The workspace test command completed successfully, including the two focused process tests.

## Limits

This evidence does not prove Ready episodes with pending operations can be resumed, canonical
sideband recovery, host or game settlement, provider accounting across a crash, gateway lease
ownership, or live gameplay. Those remain separate recovery work and require the corresponding
authorized runtime and host evidence.
