# Phase 3 harness completion report

Date: 2026-09-11. This is the harness-side bounded policy record for the additive
`ascension.context-memory.*.v1` namespace. Harness implementation code is pinned to
`3cb72968bee8943e89158a28cec27d7b88e1ce91`; the companion target implementation is pinned to
`8e52da837ae0a23cea18d7cd3d5164765911e6b6`. The record is an implementation handoff, not a live
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
stopwords, bounded terms, and held-out label isolation. The executable fake-peer and adapter-demo
tests use only synthetic bytes and loopback stdin/stdout; the adapter demo crosses the built target
CLI, fake peer, review, Phase 2 commit-held approval, and first-resume paths.

The synchronized 90-row matrix records 89 `executed_synthetic` and 1 `blocked` row. The remaining
blocked row is the unavailable native three-level reviewer lane.

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
or trajectory claim. The current run emitted `samples=32`, `corpus_entries=64`,
`corpus_bytes=1782`, `baseline_p50_us=19`, and `memory_retrieval_p50_us=443`.

The executable adapter demo also passed with target revision `8e52da8`, harness revision
`3cb7296`, zero provider calls, zero game launches, zero external requests, and zero unauthorized
processes. Its exact digests and binary hashes are in
`docs/evidence/phase3-adapter-demo-20260911.json`.

## Limits and blockers

The harness has no game or host access, provider credentials, arbitrary process/network path, or
browser storage. The fake peer reports a bounded source manifest and output digest but never returns
raw source bytes or invokes management tools. Durable local persistence and policy tests do not prove
production WAL/temp/backup operations, provider quality, native action effects, or deployment.

The remaining mandatory unavailable lane is an independent native three-level reviewer. The current
target browser audits passed with the
documented Chromium library environment at target revision `f22296225c9e6b5a36004d1d689f9e27384920ba`,
recording zero external/provider/game effects and no browser persistence. Harness PR #70 was merged
after its `policy` and `Rust quality gates` checks passed; its merge commit is
`8d771f128bc0ba13071063425c9a852bac2c40c1`. Target PR #3 was also merged externally with merge
commit `114b3e5ae28cd60d9dafc421711dee859b602c4d`. No release, deployment, provider call, game
launch, or unrelated write was performed. A daemon-backed Codex `0.154.0` recheck with
`multi_agent_v2` explicitly enabled created one real `gpt-5.6-luna`/`max` depth-1 child with
observed parentage, but that lead exposed no native child-spawn, reservation, or messaging controls.
No depth-2 coordinator or depth-3 leaf was created; the root and lead were completed and archived.
The earlier `0.153.4` command-center and standalone probes remain in the preflight artifact for
comparison. A second 0.154.0 probe using the catalog's Astra `ultra` automatic-delegation mode
produced the same depth-1-only result.
