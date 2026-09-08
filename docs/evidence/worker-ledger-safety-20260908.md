# Integrated worker ledger safety candidate

Classification: confirmed local synthetic SQLite, test, and build evidence.
Tested source: `9e6adc268a0f1d0a7bfa080e054e716ab30bc9a0` on
`codex/harness-worker-ledger-integration`. Independent H60 review is pending;
this candidate has not been promoted to the primary runtime integration branch.

The H54 repair (`c2cbe6ea2fcdfd6b36039a39ecc21c462942ee4a`) was integrated
with the root migration corrections and independent restart/reference tests.
It adds single-use, exact-store execution permits, bounded terminal projections,
consistent terminal-reference limits, failed-job retention, and worker reboot
control fencing. See [the source contract](../worker-ledger-safety.md).

## Regressions executed after integration

Both previously failing root regressions now pass:

- A surviving watchdog can explicitly authorize the replacement worker with a
  newer control sequence; the old worker remains rejected and restart itself
  remains stopped, unauthenticated, and non-admitting.
- Terminal references of 512, 513, and 1024 UTF-8 bytes commit to a real temporary
  SQLite database, survive close/reopen, acknowledge, and survive another reopen
  followed by duplicate acknowledgment. These tests use synthetic records only.

## Root validation

All commands returned exit 0 against the tested source, using a distinct build
directory for this worktree:

```text
cargo test --locked --offline -p sts2-harness --test worker_control_restart --test worker_terminal_reference
cargo test --workspace --all-targets --all-features --locked --offline
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo run --locked --offline --package repo-policy -- --strict
cargo build --workspace --all-targets --all-features --locked --offline
```

Strict policy inspected 479 sized files with zero warnings and errors before
this evidence document was added. Focused regression result: 2 tests passed.

SHA-256 of selected tested files:

| File under `crates/harness/` | SHA-256 |
| --- | --- |
| `src/execution/store_worker.rs` | `0b8102be91e5e968849942ed7f64944369f47c09ddca6fd41c7981b551dad988` |
| `src/execution/store_worker_queries.rs` | `0ecf2a798f5c0b15123f1b99dd6ec45d74e935c3e2638c70c2cb95cb2b6056c1` |
| `src/execution/types_worker.rs` | `0d74f9964527c66687c63f516021c5a06f3b7ac2628bcd0c4afd4420cf4cfe95` |
| `tests/worker_control_restart.rs` | `b5134a12d11994b1601b3f363f9bb0e816c90d63de70f8ff207cb1f1cb7908c5` |
| `tests/worker_terminal_reference.rs` | `85a62cf4a1c53f1ba976b1895441d65418d5acc00a473a98e490a0d5c6364a06` |

## Duplicate completion correction

Subsequent tested source: `5e79bca` (the earlier file hashes above remain scoped
to `9e6adc2`). A new file-backed corruption regression first failed with exit
101: after a completed worker receipt, changing the durable completion status
to the core-valid `quarantined` value was incorrectly accepted by duplicate
completion recording. The projection returned an error, but the duplicate check
only rejected successfully projected unequal values.

The correction requires a successful equal projection; missing, failed, and
unequal projections reject as corruption. The new regression and all 46
execution-store tests pass. Root also reran the full workspace/all-target/
all-feature locked offline test suite, formatting, workspace Clippy with warnings
denied, and strict policy (480 sized files, zero warnings/errors); all returned
exit 0. Independent acceptance of this follow-up remains pending.

## Validation limits

This evidence does not prove authenticated worker endpoint wiring, continued
episode execution, native service behavior, power-loss durability, live game
recovery, cold reboot, or soak completion. No push, merge, service installation,
game/provider launch, host reboot, or release activation occurred in this gate.
The earlier migration evidence remains an exact-source historical record,
not a current acceptance decision for the repaired ledger.
