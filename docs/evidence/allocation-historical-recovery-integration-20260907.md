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

## Operation identity integration

The independently reviewed UUIDv4 candidate
`727e4a8c2cb8fd3fc398b6d84aa54374a605d8d4` was applied as
`d06284c` over the documented allocation/recovery integration. On that combined
source, root reran format, Clippy with warnings denied, full workspace/all-target/
all-feature tests, build, and strict policy: all exited 0. Policy reported
431 sized files and zero warnings/errors. Operation IDs are now created with
the pinned UUIDv4 generator before dispatch; this does not substitute them for
authoritative state identities. The fresh-runner test uses separate instances
in one process, not an OS restart.

## Remaining integration blockers

The production release and invalid-allocation cleanup paths still use a non-UUID
correlation that the signed host-lease control path rejects. Exact raw legal-actions
catalog retention still requires its separately owned repair. Current allocation authority must be wired to
new durable operation contexts without overwriting historical identities.
Historical receipt settlement also requires a verified fresh-observation handoff
before the episode runner can continue; isolated parser tests do not prove it.

These passing gates do not resolve those findings. No live provider/game run,
service installation, native Windows execution, reboot, release activation,
remote publication, merge, or soak is established by this integration batch.
