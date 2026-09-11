# Phase 3 harness memory policy

`crates/harness/src/context_memory.rs` is the owner-local pure policy for the additive
`ascension.context-memory.*.v1` contracts. `MemoryCorpus` performs bounded immutable admission,
scope/branch/cutoff membership, deterministic lexical ranking and source revocation. `MemoryEntry`
keeps evidence status, authority, occurrence identity, source digest, observed/admitted sequence
and bounded derivation parents. Protected host observations, legal catalogs and unresolved
operations are never replaced by historical memory.

`MemoryCorpus::exact_extract` produces source-linked claims. `SummaryJobStore` admits explicit
review-required jobs, validates a source manifest and preserves `outcome_unknown` after a possible
external write. `FakeSummaryPeer` is a bounded test adapter with no game, control, shell or
arbitrary-network port. `MemoryCorpus::select` packs optional sources only after mandatory bytes,
pins and wrapper overhead are accounted for. `ApprovalStore` binds the selection, source and Phase2
prepared-manifest identities; commit remains held until explicit resume and revocation invalidates
old approvals.

The offline harness also includes a SQLite adapter (`DurableMemoryStore`) that keeps source
metadata separate from XChaCha20-Poly1305 ciphertext, a revocation table, an atomic projection
rebuild, a generation-aware retrieval cache, bounded unknown-work reservations, resumable migration
state, finite retention accounting, atomic map-bundle swaps, immutable review revisions, a private
held-out evaluation partition, per-attempt usage accounting, and one-shot exact resume fencing.
`context-memory-cli` exercises metadata listing, local search, exact extraction, and policy preview
against a fixed synthetic corpus; `context-memory-peer` is a line-delimited fake provider boundary
that reports the exact source manifest and never returns raw source bytes; `context-memory-bench`
prints a bounded baseline/retrieval measurement with summary and maintenance costs set to zero.
The independent oracle tests keep hand-labelled membership, tie order, Unicode normalization and
held-out scope checks outside the ranker implementation. These are deterministic component lanes,
not live provider/game or cross-repository Phase 2 adapter evidence.

The module has no transport/database/provider dependency. Persistent encrypted storage, index
projection and native provider/game execution require separately approved adapters. Live quality,
deployment and native three-level orchestration are not inferred from these deterministic tests.
