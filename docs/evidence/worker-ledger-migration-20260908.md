# Worker ledger migration candidate

Classification: confirmed local synthetic SQLite and build evidence only.
This candidate is **not accepted for primary integration or release**.

Tested source: `7df6c8cb1dd61cfe09e32417bddb90826c68a365`.
Branch: `codex/harness-worker-ledger-integration`.
The ledger was combined with the current schema-bootstrap layout at `33e2f8f`;
root migration tests and collision repair are `54b9780` and `7df6c8c`.

## Regression and correction

A schema-6 database containing an unrecognized `worker_control` table was
previously promoted to schema 7 because worker DDL used `IF NOT EXISTS`.
The new collision regression failed against `54b9780` with test exit 101.
Worker schema creation now rejects preexisting objects: creation and version
advancement belong to the same transaction, so a committed partial worker schema
is not a valid schema-6 recovery state. Rejection retains the existing object and
does not advance its version. No database is deleted or automatically recreated.

The six schema tests pass after correction, including legacy-byte retention,
idempotent migration, a late-DDL failure rolling back new objects and version,
and rejection of the conflicting table. These are transactional SQLite tests,
not VM reboot or power-loss durability experiments.

## Executed gates

All commands below returned exit 0 with a distinct candidate build directory:

```text
cargo fmt --all -- --check
cargo test --locked --offline -p sts2-harness --lib execution::schema::tests
cargo test --locked --offline -p sts2-harness --test execution_store
cargo test --locked --offline --workspace --all-targets --all-features
cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
cargo run --locked --offline -p repo-policy -- --strict
cargo build --locked --offline --workspace --all-targets --all-features
```

Focused results: six schema tests and 38 execution-store tests passed.
Strict policy: 474 sized files, zero warnings and zero errors.
Source SHA-256:

- `schema_worker.rs`: `66c0cef66dfe947df03963c092836bae730ee655c8daf85c8fb0de423c1695fd`
- `schema_tests.rs`: `7b6dc1b0822b724207395c30e99ca229ccacf1d6f0c39b880121d9c298109b12`

## Outstanding acceptance blockers

Independent H49 rejected the underlying ledger candidate: admission/start APIs
do not distinguish the single execution winner, terminal BLOB reads materialize
before their bound is checked, terminal references disagree with the wire limit,
and failed-job state needs correction or explicit contract resolution. H54 owns
those repairs. H55 separately reviews this migration delta. Passing migration
checks do not waive either review or prove authenticated endpoint/runtime wiring.

No service installation, game/provider launch, host reboot, release activation,
remote push, or merge is evidenced by these commands.
