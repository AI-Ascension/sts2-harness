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

The module has no transport/database/provider dependency. Persistent encrypted storage, index
projection and native provider/game execution require separately approved adapters. Live quality,
deployment and native three-level orchestration are not inferred from these deterministic tests.
