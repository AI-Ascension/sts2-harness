# Context-control store evidence — 2026-09-10

This record covers the opt-in `ContextControlStore` companion seam at the Phase 2 harness branch
head. It is local source/component evidence; it does not establish native target-console, provider,
gateway, game, deployment, or soak behavior.

The context-control suites at source revision `59a9752de03321168e2834f939f93850e1db69d9` have
12 tests: the original renderer/control suite has six tests, including a renderer
digest-substitution regression, and `crates/harness/tests/context_control_races.rs` adds six
bounded admission, settlement, unknown-operation, plan-fencing, boundary, and identifier tests.
The focused migration suite is
`crates/harness/tests/context_control_migration.rs` and has eight
tests:

- encrypted journal reopen retains the Phase 1 snapshot digest and opaque outbox facts;
- wrong keys and an all-zero key fail closed without plaintext fallback;
- a commit failpoint rolls back the active control state and a later retry succeeds;
- a partially marked additive schema is repaired without manufacturing a journal;
- a legacy reader refuses management-active state and accepts explicit disabled state;
- immutable Phase 1 snapshot identity and a WAL-checkpointed backup survive reopen; and
- newer schema markers and tampered journal digests fail closed; and
- a replacement owner fences the old live handle while a wrong key cannot evict the rightful owner.

The renderer regression recomputes SHA-256 for selected, note, and objective bytes and rejects a
same-reference content substitution before the bridge can serialize it. This closes the immutable
content-digest check for the companion prepared-input seam.

The `crates/harness/tests/phase2_recovery.rs` fixture adds three recovery assertions: an interrupted
resume remains unknown and denies a new input, a provider write timeout retains an ambiguous
operation without retransmission, and a game dispatch is reconciled under the original operation
identity.

The migration suite also verifies replacement ownership: a second authenticated store handle
claims a per-handle SQLite fence, increments the recovered controller incarnation, and causes the
older live handle's write and status read to return `Fenced`. A fenced handle cannot reclaim the
token for its lifetime. This exercises P2-F064 at the local store boundary; it does not claim OS
process termination or native deployment behavior.

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
cargo test --locked --test context_control_migration       pass (8/8)
cargo test --workspace --all-targets --all-features --locked pass
```

The target console capability fact is now backed by its own local encrypted fixture; this companion
record still does not claim native or production storage. Native filesystem crash behavior, OS
process termination, prepared-provider claims, ambiguous external writes, and live migration remain
outside this evidence record.
