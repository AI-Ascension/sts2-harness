# Phase 3 harness completion report

Date: 2026-09-10. This is the harness-side bounded policy record for the additive
`ascension.context-memory.*.v1` namespace. The implementation commit is `b923192`; its target
consumer commit is `9b69951`. The evidence refreshes are pinned to harness head
`b9a7283ef905005dfd7d34b2be2ff611c859b5cc` and target head
`b59f10823e5f99de4012b0aa097efd62a9260a01`. It does not claim a live provider, native game, deployment, or
three-level native development hierarchy.

The implementation is in `crates/harness/src/context_memory/` with the public include wrapper at
`crates/harness/src/context_memory.rs`. It covers scoped/digest-bound source admission, bounded
provenance, causal cutoffs, deterministic Unicode lexical ranking, exact citation spans, explicit
fake summary jobs, independent review/admission, whole rendered-input selection, Phase 2 prepared
manifest binding, held approvals, revocation fences, map generation checks, role ACLs, and aggregate
telemetry redaction. The target console consumes these outputs through its own `/v3/memory` facade;
the harness does not add a second target store or scheduler.

The synthetic test `crates/harness/tests/phase3_memory.rs` exercises six cases: late/future/sibling/
private/protected filtering, exact extract/review/admission, bounded fake summary input and unknown
outcome, held approval/explicit resume/revocation, closed query/projection lag, and map/ACL/
telemetry boundaries. The full 90-row requirement and failure mappings live in the target report's
companion artifacts; this repository records the same matrix for the draft handoff. Its final matrix
has 44 `executed_synthetic`, 40 `unverified`, and 6 `blocked` rows.

Strict policy, formatting, Clippy, Phase 3 tests, and the locked workspace test suite are required
gates. They passed for this implementation: policy, formatting, Clippy, 6 Phase 3 tests, and the
deterministic serial workspace run (`-- --test-threads=1`) with 171 tests passed and 1 ignored. The
fake peer is synthetic external-boundary evidence only: it has
no process, game, management, shell, arbitrary-network, credential, or hidden-reasoning capability.
Durable encrypted index/WAL/backup storage, restore/downgrade, live adapter fidelity, native
game/action lineage, evaluation metrics and resource tradeoffs are unverified and remain separate
adapter work.

The existing Phase 2 control authority remains the only revision/commit/resume owner. Summary
generation cannot approve itself; review/admission cannot activate a revision; selection cannot
resume a run; and revocation denies derived dependents before cleanup. These seams are intentionally
pure and transport/provider independent so the next adapter can be tested with process/network
tripwires without weakening the legacy contracts.
