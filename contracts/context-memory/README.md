# Harness Phase 3 context-memory contracts

The harness owns the pure memory policy behind these additive `ascension.context-memory.*.v1`
contracts. `crates/harness/src/context_memory.rs` validates closed payloads, source scope and
digests, observed/admitted cutoffs, bounded provenance, deterministic lexical ranking, exact
extracts, isolated summary jobs, review/admission, whole-input selection and Phase 2 approval
binding. Indices and provider peers remain disposable projections at the edges.

The older `ascension.context-control.*.v1` profile is retained unchanged. Memory selection is
always prepared and approved through the existing Phase 2 authority; commit remains held and only
an explicit resume may release a gameplay attempt. The fake summary peer is an external-boundary
test adapter and does not establish live provider quality.

Artifact source: Phase 3 package, consumed against target `0c1f402b0b6c7f0ab79eb369a649286a46482e3a`
and companion `8674874feccfbf995ed0aa5a8ec8390d9dac137b`. The current branch is draft-only.
