# Implemented Phase 3 harness handoff

Date: 2026-09-10. The Phase 3 memory policy is additive to the existing harness branch
`phase2/context-editing`. The implementation commit is `b923192`; the companion target pin is
`9b69951`.

`crates/harness/src/context_memory.rs` is a bounded include wrapper; its source files under
`crates/harness/src/context_memory/` keep each policy unit reviewable and within repository size
limits. The module owns the context-memory contracts and policy, while the target console owns only
its authenticated facade and presentation.

The corpus (`corpus_data.rs`, `corpus.rs`) validates scope, source digests, causal sequence and
generation, bounded parents, idempotent admission, atomic publication failpoints, immediate
revocation, recursive dependent fences and bounded cleanup. Retrieval (`retrieval.rs`,
`retrieval_helpers.rs`) applies scope/branch/cutoff/generation/status/expiry/protection/revocation
before deterministic Unicode lexical scoring and stable ties. Query identity is local response
correlation and is omitted from the closed query wire shape; snippets and private content are not
serialized.

Extraction/proposals (`retrieval_types.rs`) cite exact immutable byte spans. Review (`review.rs`)
binds proposal/source/revocation identities and cannot create an active Phase 2 revision. Summary
jobs (`jobs.rs`, `summary_store.rs`) are bounded and idempotent, require explicit generation
permission, capture exact source bytes at the fake peer, and retain ambiguous provider writes as
`outcome_unknown` without retry. Policy/selection (`manifest.rs`, `selection.rs`) rejects silent
cross-scope or automatic-summary activation, preserves pins and mandatory bytes, measures the
rendered whole, records bounded exclusions, and binds the Phase 2 prepared-manifest digest.
Approval (`approval.rs`) remains `committed_held` until explicit resume; capability/ACL/map/
telemetry records are in `capabilities.rs` and `map.rs`.

## Contract and test table

| Seam | Owner | Contract/effect | Evidence | Extension risk |
| --- | --- | --- | --- | --- |
| Entry/provenance | harness | `entry.v1`; source admission | companion phase3 tests | durable store adapter still required |
| Retrieval | harness | `query.v1` → `retrieval.v1`; local read/no inference | scope/tie/projection tests | index projection and cache persistence unverified |
| Extract/review | harness | proposal/review v1; review-only | exact extract/admission tests | independent reviewer service unverified |
| Summary job | harness provider port | summary-job v1; explicit generation only | fake peer and unknown test | live provider/process boundary unverified |
| Selection/policy | harness | policy/selection v1; whole bytes | selection tests | Phase 2 serializer adapter unverified |
| Approval/revocation | harness + existing Phase 2 | approval/revocation v1; held/epoch fenced | approval/revocation tests | restore/downgrade transaction unverified |
| Map/telemetry | harness | generation-fenced map; redacted aggregate telemetry | map/ACL/telemetry tests | executable map artifact lane unverified |

The module has no game or host access, no provider credentials, no arbitrary process/network path,
and no browser storage. The fake peer is synthetic boundary evidence only. A future target adapter
must consume these policy outputs through declared ports and preserve the existing Phase 2 prepared
bytes, pause/commit/resume, plan epoch, and action-lineage authorities.
