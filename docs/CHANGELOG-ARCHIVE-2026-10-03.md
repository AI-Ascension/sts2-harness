# Changelog archive: 2026-10-03

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link
paths, which are corrected so they resolve from this directory; this file is a verbatim record,
not a supported release or a second normative changelog.

- **Bind bounded-region admission to the plan's region and planner-profile identity.** A
  follow-up review of the bounded parallel analysis route found that
  `workflow::bounded_region::admit_bounded_region` checked the parallel cap, the region's
  admissible operations and the plan's structural validity but never compared the plan's
  `region_id` / `planner_profile_ref` against the region it was admitted for, unlike the sibling
  `DynamicPlanRegistry::accept`. Because both the plan and the region are caller-supplied, a plan
  that named a different region or planner profile was admitted whenever its operations fell inside
  the caller-supplied `allowed_operations`. Admission now refuses such a plan with a dedicated typed
  reason, `BoundedRegionRefusal::PlanIdentityMismatch`, before any branch is dispatched, and the
  module contract states that base-revision continuity remains `accept`'s responsibility because the
  region does not carry the base digest or revision and the runtime supplies only the workflow
  limits. Compatibility: tightening — this route has no in-repo caller, and a plan that names its own
  region and profile is unaffected. Source-only: no native effect. Refs #465.

- **Hold the Exo request-level identity to the published wire width.** The published
  `sts2.exo-bridge-wire-v1` schema binds `decision_request.model_execution_id` and `.state_id` to
  `$defs/id` (`maxLength` 512) and the protocol validator admits the same, but the lifecycle
  manifest refused both at 128, and the internal identities the owner mints from them
  (`lifecycle-binding-`, `lifecycle-prepared-`, `provider-execution-`) were refused above an
  effective 109 bytes — a ceiling written in no schema. The two request-level fields are now
  validated against the published width while every envelope/control identity keeps its own
  128-byte bound, so a host that follows the published schema is no longer refused before
  dispatch, and the two refusal vocabularies that depended on how wide the value was are gone.
  Internal identities carry a digest of the request identity rather than the identity itself, so
  their width no longer grows with it. Compatibility: additive at the wire — it only admits
  identities that were previously refused and changes no published schema; every refusal stays
  fail-closed before dispatch. Refs #458; see
  [ADR 0077](decisions/0077-exo-request-identity-width.md).

- **Correct the bounded-analysis documentation contract.** The `workflow::bounded_region` module
  doc cited a `BoundedAnalysis` type defined on no revision across all 64 remote refs and the bare
  `[`AnalysisValue`]` beside it — both dangling intra-doc links — and claimed a declared adaptive
  region now runs "on the budget-reserved bounded route instead of only through a caller-supplied
  adaptive executor", which the shipped wiring does not do: `DynamicRuntime::step()` still
  dispatches a declared `adaptive_region` node to `DynamicExecutorPort::execute_adaptive`, and
  `execute_bounded_region` has exactly one caller, a test. No gate saw it — no workflow runs
  `cargo doc`/`rustdoc`, the crate denies no `rustdoc::broken_intra_doc_links`, and the private
  module is skipped by a default rustdoc run — so it built, linted and tested green while its own
  contract statement was false. The module doc now names the real entry point
  (`DynamicRuntime::execute_bounded_region`) and its inputs, states the node route is unchanged and
  the bounded route has no in-repo production caller, and records the two bindings the route
  deliberately does not make: base-revision continuity, and the caller-supplied region against the
  workflow's declared `adaptive_region` node, which no accessor exposes (the #465 review left it a
  design extension). The public `execute_bounded_region` rustdoc is corrected to match — the region
  is *caller-supplied* and is checked against the plan
  (`BoundedRegionRefusal::PlanIdentityMismatch`) but never a declared node. Compatibility: no code,
  schema, route, refusal, bound or digest change; documentation only. Source-only: no native
  effect. Refs #470.
