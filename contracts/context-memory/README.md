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

The `capabilities.v2` profile adds the effective-limit descriptor without changing the portable
policy schema. `policy.schema.json` defines portable syntax ceilings, including a 65,536-byte optional budget.
They are not execution promises. The owner publishes the selected policy revision's effective
limits in `capabilities.schema.json`; the current harness profile limits the optional budget to
8,192 bytes. A schema-valid policy above an effective limit remains inspectable but is rejected at
admission and must be replaced through explicit approval rather than silently clamped.

Artifact source: Phase 3 package, consumed against target implementation `9b69951` and companion
implementation `b923192`. The current branch is draft-only.
