# ADR 0014: harness-owned bounded context memory

Status: accepted for the Phase 3 draft branch. Owner: harness maintainers.

`context_memory` is a pure policy module beside the existing context-control and provider ports.
It admits immutable owner-scoped entries, keeps observed and admitted sequence axes distinct,
validates a bounded derivation graph, and marks roots and all descendants unusable immediately on
revocation. Content is retained only while the feature is enabled; cleanup removes derived bodies
without rewriting historical manifest identities.

Retrieval filters scope, branch, cutoff, corpus generation, expiry and revocation before a
deterministic lexical ranker runs. Query bytes, terms, candidates, results, snippets and optional
selection bytes are bounded. The response reports projection degradation and zero model calls;
scores are explainable relevance signals.

Exact extracts cite immutable byte spans. Abstractive generation is an explicit durable job with
an idempotency key, source-manifest digest, review-required output and an outcome-unknown state
that cannot be retried automatically. A fake summary peer records only the bounded request at the
external boundary and has no game, control, shell or arbitrary-network capability.

Selection manifests preserve mandatory input and pins, measure wrapper bytes, carry unknown token
measurements honestly and bind to the existing Phase 2 prepared manifest. Approval state transitions
are `preview_ready -> committed_held -> explicit resume`; revocation or expiry fences the approval.
No memory operation creates a gameplay action or resumes a run.
