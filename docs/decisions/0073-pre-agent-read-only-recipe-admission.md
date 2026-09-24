# ADR 0073: Admit a bounded pre-agent read-only recipe

Status: accepted for the harness-owned, source-only slice of issue
[#97](https://github.com/AI-Ascension/sts2-harness/issues/97) — a bounded, versioned recipe of
approved read-only reads that the harness performs before provider dispatch. It performs no read,
starts no process and spends no provider call; collection execution, durable provenance and the
Studio round-trip are separate slices, and native mapping stays gated by the separately authorized
real-tool lane. It is ratified when the change carrying it merges.

## Context

Issue #97 requires an authored workflow to fetch game state, legal actions and other approved inputs
programmatically before inference, with no model tool call needed to gather them. Runtime-v3 already
calls `sts2.observe` and `sts2.legal_actions` automatically, but there is no harness-owned contract
that fixes *which* reads a recipe may declare, in what order, at which revision, and under what
bounds — so nothing could refuse a recipe that names a mutating tool, an unapproved revision or an
impossible ordering before any effect.

Three failure modes had to be excluded by construction:

1. a tool description claiming "read-only" while the harness-owned classification says mutation;
2. an ordering that is not decidable (a dependency on a later or unknown step, or a cycle);
3. a required read silently satisfied by an optional one, so a waiver could unblock dispatch.

## Decision

A new module `crates/harness/src/recipe/` owns the contract, split so each file stays inside the
production size budget, and is additive to the existing observation/legal-catalog paths:

- **Identifiers and revisions (`ids.rs`).** `RecipeId`, `StepId` and `ToolId` accept only a
  non-empty, ≤96-byte, portable shape (`[A-Za-z0-9._:-]`), so a recipe cannot smuggle a path, URL or
  control character into a tool mapping. `RecipeRevision` and `ToolRevision` are explicit numbers.
- **Fixed catalog (`catalog.rs`).** `ApprovedTool` pins one tool to exactly one revision, a
  harness-owned `ToolClass` (`ReadOnly` or `Mutation`) and the argument-schema identifier that
  revision owns. A read-only claim in a description is not consulted.
- **Authored shape (`definition.rs`).** `RecipeStep` declares a tool revision, a typed argument map,
  backward-only dependencies, required/optional outputs, a timeout and a freshness horizon.
  `RecipeLimits::standard` fixes the bounds: 32 steps, 16 arguments, 8 dependencies, 8 outputs, 512
  argument bytes, 60 s timeout, 1 h freshness.
- **Effect-free admission (`admission.rs`, `error.rs`).** `admit_recipe` runs a fixed check order —
  recipe revision, result schema, non-empty steps and step bound, then per step: duplicate identity,
  zero tool revision, catalog membership, read-only classification, argument schema, arguments,
  dependencies, outputs and limits — so the first failing property is reported. A dependency must
  name an *earlier* step, which makes declared order a valid topological order and refuses cycles,
  self-dependencies and forward references without a separate sort. A required step may not depend on
  an optional one. `AdmittedRecipe` is opaque and exposes only the ordered steps.

## Consequences

- A recipe with any prohibited step is refused whole; admission never partially accepts.
- Bounds are structural, so an over-large or cyclic recipe is refused before any read is attempted.
- The refusal vocabulary carries only structural identity, never a supplied argument value or game
  text, so a refusal can be logged without leaking authored content.
- Every string field on a refusal that admission can populate is either a value that already passed
  its bounded shape check or a typed identifier; `ZeroRevision.field` is a static literal and
  `InvalidSchema.step` is only ever built as `None` today. `InvalidOutput` is the one refusal raised
  *by* a failed check, so it names the offending **slot index** rather than the rejected name; a
  refusal never carries text that failed admission. Should T2/T3 add authored intake, tighten
  `InvalidSchema.step` to a typed identifier at the same time.
- This slice deliberately maps no tool and persists nothing. Collection execution and provenance
  (issue #97 T2/T3), the Studio recipe round-trip, and the separately authorized real-tool lane for
  native mapping remain open and are not claimed satisfied here.

## Amendment 2026-09-24: `InvalidOutput` no longer carries the rejected name

The consequences above were the contract from the start, but one variant did not meet it:
`RecipeAdmissionError::InvalidOutput` stored the raw `OutputSlot.name` that had just failed
`is_identifier` — by construction a value *guaranteed* to violate the ≤96-byte, `[A-Za-z0-9._:-]`
bound, and free to carry control bytes or arbitrary authored text. Every other string field admission
populates holds a value that passed its shape check (`DuplicateOutput.output`) or a typed identifier
(`step`, `tool`, `dependency`), so this was the single exception to a documented invariant.

Reachability today is Rust-API-only: `RecipeDefinition`/`OutputSlot` carry no `serde` intake and the
module has no consumer outside `recipe/` (plus its test), so no untrusted document can reach the
error yet. The exposure would begin when T2/T3 add authored or Studio-facing intake, at which point a
recipe document could place arbitrary bytes into a refusal string a log or UI renders verbatim.

`InvalidOutput` now reports the offending **slot index** (`slot_index: usize`) and keeps the rejected
name out of the refusal, matching how the other variants are built: diagnostic value ("step `x` slot
3 is not portable") without unbounded text. Classification: **safety-correction** — the variant is
public, but the crate is consumed only by its own workspace, and no record, schema, route, wire
format or digest changes; admission behaviour (which recipe is refused, and where in the fixed order)
is unchanged.
