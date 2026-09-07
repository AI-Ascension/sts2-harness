# Allocation and historical-recovery component integration

Classification: confirmed Linux component checks; recovery release and complete
cross-component operation recovery remain unverified.

Tested source: `e3508b06a5de11056416f8a34516abd1314d24cf`.
The integration combines historical-response validation at
`0bbd873fccec9887f59189f7a26261dc6ed5a5b1` with allocation consumer
`0450a40ba3612adaac23885a99d019071ea79448` and provenance repair
`c6497f655970dd98ca6408afb67b7bb6354c921e`. The latter two were applied as
`7347ba0` and `e3508b0` in an isolated integration branch without conflicts.

Root independently ran the following with locked dependencies and a target
directory unique to this integration worktree:

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | exit 0 |
| `cargo test --locked -p sts2-harness --bin sts2-harness-runtime allocation` | exit 0; 17 tests |
| `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| `cargo run --locked --package repo-policy -- --strict` | exit 0; 429 sized files, zero warnings/errors |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| `cargo build --workspace --all-targets --all-features --locked` | exit 0 |

The allocation schema SHA-256 is
`ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b`.
All four imported artifact blobs match the producer revisions named by the
artifact provenance README. The component launch test uses a synthetic gateway;
its successful release response is not evidence of signed host revocation.

## Remaining integration blockers

The production release and invalid-allocation cleanup paths still use a non-UUID
correlation that the signed host-lease control path rejects. Production operation
identity creation and exact raw legal-actions catalog retention also require
their separately owned repairs. Current allocation authority must be wired to
new durable operation contexts without overwriting historical identities.

These passing gates do not resolve those findings. No live provider/game run,
service installation, native Windows execution, reboot, release activation,
remote publication, merge, or soak is established by this integration batch.
