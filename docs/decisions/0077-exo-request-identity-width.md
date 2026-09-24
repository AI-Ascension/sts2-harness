# ADR 0077: The Exo request-level identity is held to the published wire width

Status: accepted for the harness-owned, source-only slice of issue
[#458](https://github.com/AI-Ascension/sts2-harness/issues/458). It calls no provider, launches no
game and needs no native evidence to close. It is ratified when the change carrying it merges.

## Context

`sts2.exo-bridge-wire-v1` defines two identity kinds on purpose: `$defs/id` (`minLength` 1,
`maxLength` **512**) for the request-level `decision_request.model_execution_id` and `.state_id`, and
`$defs/control_id` (**128**) for the envelope `request_id`/`turn_id`. The protocol validator matches
the schema (`exo/protocol/request_validation.rs`: `valid_id` admits up to 512), the execution store
validates `execution_id` at 512, and the runtime-v3 observation rule that ADR 0056 cites is 512. The
lifecycle manifest alone bounded both request-level fields at 128.

Because the owner also mints internal identities *from* the request identity, the effective ceiling
was narrower still: `lifecycle-binding-{id}` (18-byte prefix) and `lifecycle-prepared-{id}` (19-byte
prefix) were re-validated at 128 by `provider_session::valid_id`, so a 110-byte identity produced a
129-byte prepared identity and was refused — a number written in no schema and no ADR. The refusal
arrived as two different error vocabularies depending only on how wide the value was: `Invalid`
above 128, `Held` in 110..=128.

The narrowing alternative was to keep 128 and shrink the published 512, but that is a breaking wire
change for a bound no known host approaches, and it contradicts the repo's own accepted 512
statements. The 128-byte sentence in ADR 0017 governs *envelope* control identities, not the
request-level `state_id`.

## Decision

The request-level identity width is the published wire width: **512 bytes**, the `$defs/id` bound.
Envelope and control identities stay at **128** (`$defs/control_id`, ADR 0017).

- `InvocationManifest::validate` validates `execution_id` and `authority.state_id` with `wire_id`
  (512) and every other identifier with `id` (128). The manifest, its lifecycle entries and the
  owner journal share these two predicates.
- The schema, the protocol validator, the execution store and the manifest now agree at 512 for
  exactly these two fields, and every envelope/control field agrees at 128.
- Internal identities minted from the request identity no longer embed it. `derived_ids` formats
  each one as its prefix plus the lowercase hex SHA-256 of the identity, so a derived value is
  82–83 bytes regardless of the admitted width and every one stays inside the 128-byte internal
  bound. The broker `phase3_selection_id` and the idempotency key carry the same token.

The direction is additive at the wire: it only turns previously-refused identities (110..=512) into
admitted ones. It does not narrow the published schema, and it exposes no new or changed identifier
to the wire.

## Consequences

- A host that follows the published schema and ships any identity up to 512 bytes now succeeds. The
  hidden 109-byte ceiling and the two unrelated error vocabularies (`Invalid` versus `Held`) are
  gone.
- Derived identifiers change value shape (prefix plus digest) but not their role. They are
  deterministic, injective and matched by equality inside one owner session, and broker lookup stays
  exact. Records are correlated by `execution_id`, not by a derived id; the broker is
  per-invocation, so no persisted correlation depends on the old shape.
- The internal 128-byte bound is unchanged, so this is not an authority increase: every refusal
  stays fail-closed before dispatch, and the change touches only the width predicate and the
  derivation of internal identifiers.
- The managed/context-control render path keeps its own 128-byte bound
  (`context_control/render.rs`), which is *published* in `contracts/context-control/boundary.schema.json`
  and `preview.schema.json`. Widening it would be a cross-repo contract change and is out of scope
  for this source-only slice; it is recorded as a residual, not silently aligned.

## Residuals

- `context_control::render::validate_request` and `context_control::types::valid_id` still bound
  `state_id` at 128. Their `state_id` is the published context-control boundary value, so changing
  it needs a coordinated contract revision in `contracts/context-control/`.
- `provider_session::owner_journal::semantics` still pins the *derived* internal identities at 128
  through `types::id` and `provider_session::valid_id`, which is the point: the internal bound is
  the thing the derivation must respect, and the digest makes every admitted width satisfy it.
- `runtime_support/runtime_v3_durable_operations.rs` still mints `provider-execution-{execution_id}`
  by concatenation. It is safe today only because that path's identity is the numeric allocator's
  `model-execution-<n>`, so it is recorded here as the same shape this ADR replaces rather than
  silently aligned; widening it would need the same digest discipline.
