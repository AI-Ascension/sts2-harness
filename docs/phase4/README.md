# Phase 4 persistent-provider fixture

`crates/harness/src/provider_session` contains the typed policy, broker, strict frame parser,
owned stdio transport and compiled `provider-session-peer`. It is intentionally separate from the
stateless Exo/Ollama adapters. The broker keeps evaluation forks game-free, compaction held and
unknown outcomes fenced. The target console consumes only its metadata projection.

Run the focused lane with:

```text
cargo test --locked -p sts2-harness --test provider_session
cargo test --locked -p sts2-harness --test provider_session_snapshot
```

The lane is synthetic and offline. It does not certify an installed Codex binary, provider
authentication, native persistence, gameplay settlement, or provider-side deletion. Snapshot
restore only covers the bounded local metadata journal; prepared turns require explicit
reconciliation after restart.
The fixture capability advertises native encrypted persistence as unverified, and the broker rejects
an `enabled` profile until an encrypted boundary is independently verified.
