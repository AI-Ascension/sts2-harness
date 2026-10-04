# Changelog archive: 2026-10-04

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link
paths, which are corrected so they resolve from this directory; this file is a verbatim record,
not a supported release or a second normative changelog.

- **Run the workspace doctests in CI.** No gate executed doctests: the `rust` job runs
  `cargo test --workspace --all-targets --all-features --locked`, and `--all-targets` excludes the
  `--doc` target by definition, so the repository's single doctest — the `compile_fail,E0624` guard
  proving that an external caller cannot construct an `AuthenticatedWorkerRequest` — had never been
  compiled in CI. A new `Run doctests` step runs `cargo test --workspace --doc --all-features
  --locked`, so that authority-boundary assertion (and any future doctest) is now executed and
  cannot rot silently. Compatibility: CI-only; no source, schema, route or behavior change.
  Refs #479.

- **Repair nine intra-doc-link defects and gate the class durably.** `sts2-harness` failed a
  documentation-integrity expectation its own gates could not see. Seven intra-doc links across six
  files named a type or method that does not resolve at the file's own scope — three of them
  reachable by a plain `cargo doc` — and two more named a bare `[`plan`]`, ambiguous between a
  function and a module (`benchmark_manifest::branch_experiment` and `benchmark_manifest::suite`).
  Every one names a real item elsewhere in the crate, so each was a scope/path or disambiguation
  defect rather than a stale name: `execution::types::worker` looked for `ExecutionStore` under
  `execution::types`, which re-exports only `ExecutionStoreError`;
  `management::save_profile_setup::setup` attributed `verify` to `VerifiedProfileReadback` when it is
  an inherent method of `ProfileReadback`; `provider_session::types::effective_limits` linked a bare
  `ProviderSessionPolicy`; and `context_control::membership`, `context_control::model_view` and
  `management::lifecycle` linked bare names owned by sibling modules. Each link now carries a path —
  or, for the two ambiguous `plan` links, a `()` disambiguator — that resolves. The durable half is a
  `cargo doc` step in the `rust` job with
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links --document-private-items"`, because a default
  rustdoc run skips the private modules that hold four of the seven unresolved links; no workflow had
  run `cargo doc`/`rustdoc` before, so nothing owned the class. Refs #477.

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

- **Repair nine intra-doc-link defects and gate the class durably.** `sts2-harness` failed a
  documentation-integrity expectation its own gates could not see. Seven intra-doc links across six
  files named a type or method that does not resolve at the file's own scope — three of them
  reachable by a plain `cargo doc` — and two more named a bare `[`plan`]`, ambiguous between a
  function and a module (`benchmark_manifest::branch_experiment` and `benchmark_manifest::suite`).
  Every one names a real item elsewhere in the crate, so each was a scope/path or disambiguation
  defect rather than a stale name: `execution::types::worker` looked for `ExecutionStore` under
  `execution::types`, which re-exports only `ExecutionStoreError`;
  `management::save_profile_setup::setup` attributed `verify` to `VerifiedProfileReadback` when it is
  an inherent method of `ProfileReadback`; `provider_session::types::effective_limits` linked a bare
  `ProviderSessionPolicy`; and `context_control::membership`, `context_control::model_view` and
  `management::lifecycle` linked bare names owned by sibling modules. Each link now carries a path —
  or, for the two ambiguous `plan` links, a `()` disambiguator — that resolves. The durable half is a
  `cargo doc` step in the `rust` job with
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links --document-private-items"`, because a default
  rustdoc run skips the private modules that hold four of the seven unresolved links; no workflow had
  run `cargo doc`/`rustdoc` before, so nothing owned the class. Refs #477.

- **Keep the rejected output name out of the recipe refusal.** The pre-agent recipe admission
  contract documents that its refusal vocabulary "carries only structural identity, never a supplied
  argument value or game text", and every variant met that except one:
  `RecipeAdmissionError::InvalidOutput` stored the raw `OutputSlot.name` that had just failed
  `is_identifier` — by construction a value guaranteed to violate the ≤96-byte, `[A-Za-z0-9._:-]`
  bound, and free to carry control bytes or arbitrary authored text. It now reports the offending
  slot **index** instead, matching how the other variants are built (the step and dependency fields
  are typed identifiers; `DuplicateOutput.output` passed its shape check). Reachability is
  Rust-API-only today — `RecipeDefinition`/`OutputSlot` have no `serde` intake and the module has no
  consumer outside `recipe/` and its test — so nothing untrusted could reach the error yet; the
  exposure would have begun when T2/T3 add authored or Studio-facing intake. Compatibility:
  safety-correction — the variant is public but the crate is consumed only by its own workspace, no
  record/schema/route/digest changes, and which recipe is refused (and at which point in the fixed
  admission order) is unchanged. Refs #97; see
  [ADR 0073](decisions/0073-pre-agent-read-only-recipe-admission.md).

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

- **Execute a bounded parallel analysis region through the production dynamic runtime.** The
  budget-reserved bounded route (`execute_plan_bounded`,
  `execute_plan_bounded_reserved`) had no production caller: a `DynamicRuntime` handled an
  `adaptive_region` node purely by delegating to the caller-supplied executor, so the owner's
  parallel cap, the atomic budget reservation and the per-branch join report were reachable only
  from tests. A new `workflow::bounded_region` module admits a declared region fail-closed
  *before* any branch is dispatched (cap from the workflow's own limits, admissible operations,
  then `validate_plan`), runs it on the reserved route, and reports a versioned, digest-bound
  outcome (`ascension.harness.bounded-analysis-report.v1`) that names each branch's actual joined
  state — settled, failed with its own reason token, or unknown — instead of a settled count, so
  a failed or lost branch can never be read as a success. `DynamicRuntime::execute_bounded_region`
  retains that report for a consumer; the existing node route is unchanged. The mutation clause
  holds by construction rather than by assertion: `DynamicNodeKind` is a `deny_unknown_fields`
  `analyze`/`decide` enum and the executor returns an `AnalysisValue`, so a plan naming a mutating
  node kind is refused at decode and no bounded branch can reach a game mutation. Compatibility:
  additive; two new module files, one new runtime method, no change to an existing schema, route,
  digest or node kind. Refs #98.

- **Resolve inference-profile references authoritatively at the owner, not in the browser.** A
  `decide` / `adaptive_region` reference could name a floating `profile_id` that no served catalog
  advertises, and `POST /v1/workflow-definitions/validate` returned `{"valid":true}` for it, because
  live submission admission — not definition validation — was the only thing that resolved profiles.
  A consumer wanting an immutable binding therefore had to invent a *second, stricter* rule in the
  browser and refuse publication the owner would have accepted, leaving two admission authorities
  disagreeing about the same document. Both surfaces now resolve every reference through the
  owner's served catalog with the same per-node fences live submission already used, and publish
  that decision: `profile_ref` as authored beside `resolved_pin`, the exact
  `profile_id:version:digest` it resolved to, plus its graph, node, kind and JSON path. Both fail
  closed — an uncatalogued floating id is refused with the catalog's own `inference_profile_unknown`
  rather than silently accepted, and publication refuses before creating a definition. A consumer
  records `resolved_pin`, never `profile_ref`, for immutability: a floating id can resolve to a
  different descriptor after a catalog revision. `inference_profiles` is `null` when the owner
  serves no catalog (no authority exercised — not "admissible") and empty when a catalog was served
  and the document has no reference; an owner with no catalog is unchanged. **This breaks a strict
  decoder that rejected unknown response fields.** Both surfaces publish the same decision for the
  same document, so a consumer reads the owner's verdict instead of re-deriving one. Evidence is
  synthetic/component only: in-memory owner doubles and an in-memory authoring store. No provider,
  model, credential, native host or live-owner browser run is claimed. Refs #799.

- **Run the workspace doctests in CI.** No gate executed doctests: the `rust` job runs
  `cargo test --workspace --all-targets --all-features --locked`, and `--all-targets` excludes the
  `--doc` target by definition, so the repository's single doctest — the `compile_fail,E0624` guard
  proving that an external caller cannot construct an `AuthenticatedWorkerRequest` — had never been
  compiled in CI. A new `Run doctests` step runs `cargo test --workspace --doc --all-features
  --locked`, so that authority-boundary assertion (and any future doctest) is now executed and
  cannot rot silently. Compatibility: CI-only; no source, schema, route or behavior change.
  Refs #479.
