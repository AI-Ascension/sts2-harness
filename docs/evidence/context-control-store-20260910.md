# Context-control store evidence — 2026-09-10

This record covers the opt-in `ContextControlStore` companion seam at the Phase 2 harness branch
head. It is local source/component evidence; it does not establish native target-console, provider,
gateway, game, deployment, or soak behavior.

The focused migration suite is `crates/harness/tests/context_control_migration.rs` and has seven
tests:

- encrypted journal reopen retains the Phase 1 snapshot digest and opaque outbox facts;
- wrong keys and an all-zero key fail closed without plaintext fallback;
- a commit failpoint rolls back the active control state and a later retry succeeds;
- a partially marked additive schema is repaired without manufacturing a journal;
- a legacy reader refuses management-active state and accepts explicit disabled state;
- immutable Phase 1 snapshot identity and a WAL-checkpointed backup survive reopen; and
- newer schema markers and tampered journal digests fail closed.

The store uses SQLite WAL, `synchronous = FULL`, `BEGIN IMMEDIATE`, XChaCha20-Poly1305 journal
envelopes, SHA-256 envelope/event/snapshot digests, bounded object sizes, and a schema marker
`ascension.context-control.sqlite.v1`. The test fixture uses only temporary local files and fake
control authorities; no provider, game, MCP, gateway, network, or external artifact call occurs.

Validation run from the harness worktree:

```text
cargo fmt --all -- --check                         pass
cargo run --quiet --locked --package repo-policy -- --strict  pass (0 warnings, 0 errors)
cargo metadata --locked --no-deps --format-version 1         pass
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings  pass
cargo test --locked --test context_control_migration       pass (7/7)
cargo test --workspace --all-targets --all-features --locked pass
```

The companion seam leaves the target console capability fact `durable_control_store` unverified
until the target application adopts a durable implementation. Native filesystem crash behavior,
exclusive OS ownership, prepared-provider claims, ambiguous external writes, and live migration
remain outside this evidence record.
