# Phase 3 harness completion report

Date: 2026-09-11. This is the harness-side bounded policy record for the additive
`ascension.context-memory.*.v1` namespace. Harness implementation code is pinned to
`321dbda4ed0a433ec700d9b7050d94a4b9f082ba`; the companion target implementation is pinned to
`9dfc7905b271a891b9c5d053477d4e8fb7c7d41e`. The record is an implementation handoff, not a live
provider, native game, deployment, or three-level orchestration claim.

The include-based `context_memory` module owns scoped source admission, occurrence identity,
bounded provenance, causal cutoffs, deterministic Unicode lexical retrieval, exact and explicitly
lossy extracts, review-required summary jobs, immutable review revisions, critical-fact checks,
whole rendered-input selection, Phase 2 prepared-manifest binding, held approvals, revocation
fences, generation-aware cache invalidation, and redacted aggregate telemetry. New deterministic
component lanes add encrypted SQLite metadata/ciphertext separation, revocation-first backup/restore,
finite retention accounting, resumable migration checkpoints, downgrade fences, atomic map-bundle
swaps, immutable per-attempt usage, a private held-out evaluation partition, scoped role checks,
concurrency/race coverage, and one-shot exact resume. The compiled `context-memory-cli`,
`context-memory-peer`, and `context-memory-bench` binaries exercise bounded synthetic boundaries.

Independent hand-labelled oracle tests cover causal membership, tie order, Unicode normalization,
stopwords, bounded terms, and held-out label isolation. The process tests use only synthetic bytes
and loopback stdin/stdout. They are not evidence of the unavailable target↔harness Phase 2 adapter.

## Gates

The following locked commands passed with exit status 0 on the harness implementation revision:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked -- --test-threads=1
```

The Phase 3 tests include lifecycle, persistence, oracle, mutation, race, peer, CLI, resume, and
held-out evaluation lanes. The workspace run includes the repository's existing suites and passed
serially; no ignored test was promoted to evidence. The measurement binary reports a synthetic
baseline and local lexical retrieval only, with `summary_calls` and `summary_maintenance_bytes`
explicitly zero and provider class `none_local_lexical_only`; it makes no quality, cache-benefit,
or trajectory claim.

## Limits and blockers

The harness has no game or host access, provider credentials, arbitrary process/network path, or
browser storage. The fake peer reports a bounded source manifest and output digest but never returns
raw source bytes or invokes management tools. Durable local persistence and policy tests do not prove
production WAL/temp/backup operations, provider quality, native action effects, or deployment.

The remaining mandatory unavailable lanes are the atomic cross-repository Phase 2 binding, a real
summary process/tool tripwire, target↔harness end-to-end process/network tripwires, and an
independent native three-level reviewer. Browser/current target evidence is also unavailable in the
present environment because Chromium cannot start without `libglib-2.0.so.0`. The draft PR remains
open and draft; no merge, release, deployment, provider call, game launch, or unrelated write was
performed.
