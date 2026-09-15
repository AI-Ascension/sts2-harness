# ADR 0030: Authenticated composition of the context owner's effective limits

## Status

Accepted for scoped implementation of Harness #95. No consumer adoption, renderer production
caller, control-transition enforcement or feature closure is approved here.

## Context

Two halves of the context-owner contract were already reachable, but never together:

- `GET /v1/context-bindings` returns the owner's whole catalog, whose descriptors advertise
  `ContextEffectiveLimits` (`max_items`, `max_notes`, `max_context_bytes`, `max_objective_bytes`,
  `max_control_events`); and
- `GET /v1/workflow-runs/{run_id}/context-owner-association` (ADR 0025) returns the owner's
  **current** `ContextOwnerBinding` for one run.

`ContextOwnerBinding` carries no limits of its own, and the catalog is not run-scoped. A consumer
that wants the effective limits of the run in front of it therefore had to join the two surfaces
itself, in an order and with checks nothing in the harness had established. Worse, the join is not
cosmetic: the catalog descriptor is the only thing that makes the binding's identity meaningful, and
`ContextBindingDescriptor::validate_binding` — which proves the binding does not escalate the
descriptor's grants or continuity — was applied only on the live admission path. The observable
surface could therefore disagree with the path that admitted the run, and #95's requirement that
effective limits be published as authenticated, revision-bound capability metadata had no delivery.

## Decision

Compose the two halves through a single fail-closed seam, and expose the result as a separately
versioned, read-only projection:

```
GET /v1/workflow-runs/{run_id}/context-owner-effective-limits
```

- `compose_context_owner_binding(catalog, binding)` resolves the descriptor that admits the binding
  and refuses, before any advertised value is used, a binding issued by another catalog owner
  (`409 context_owner_binding_foreign`), a binding that is not currently available
  (`409 context_binding_unavailable`), a context/node kind the catalog does not advertise or
  advertises only as disabled (`409 context_binding_unsupported`), a binding that is not the exact
  published descriptor identity (`409 context_owner_binding_descriptor_mismatch`), and any grant or
  continuity escalation the descriptor does not publish (`409 context_owner_binding_grant_escalation`,
  `409 context_owner_binding_continuity_escalation`).
- `ContextOwnerEffectiveLimitsView::compose` builds
  `ascension.harness.context-owner-effective-limits-view.v1`
  `{ schema_version, owner_id, owner_version, catalog_digest, binding_id, binding_version,
  binding_digest, context_ref, node_kind, adapter_revision, model_revision, effective_limits }`
  from a catalog that passed `validate()` — so every value is already bounded by the harness maxima
  and the descriptor/catalog digests are re-derived — and from the same binding the association
  surface reports.
- The live admission path calls the same seam (`execution_commands.rs`), replacing its inline
  owner-identity and descriptor-binding checks, so the limits a caller can observe are the limits
  the run was admitted under.

Behavior:

- current scoped `workflow:read` for the run prefix is required (`403 missing_scope`,
  `403 run_scope_denied`);
- an unknown run is `400 run_not_found`;
- an unattached port is `503 context_owner_unavailable`; an attached owner with no current
  association stays explicitly unavailable (`503 context_owner_association_unavailable`) rather
  than reporting absent limits;
- an oversized descriptor (`400 context_effective_limits_invalid`), a descriptor whose declared
  digest no longer matches its limits (`409 context_binding_digest_mismatch`) and a catalog whose
  declared digest does not match its descriptors (`409 context_catalog_digest_mismatch`) are all
  refused, so a tampered or stale owner revision cannot authorize larger limits;
- unknown query parameters and non-GET methods stay `400 route_not_found`.

## Compatibility

Additive and read-only. No existing route, request/response schema, private record, SQLite table,
digest, default or resource ceiling changes, and the harness maxima are neither raised nor lowered.
`GET /v1/context-bindings`, `POST /v1/context-bindings/bind` and the ADR 0025 association projection
keep their exact contracts. The admission refactor keeps the same error codes for the checks it
moves: owner identity (`context_owner_binding_foreign`), descriptor identity and escalation
(`context_owner_binding_descriptor_mismatch`, `context_owner_binding_grant_escalation`), and the
pre-existing `context_binding_metadata_unavailable` check still runs before the owner is asked to
bind.

## Consequences and limits

- The projected limits are the **owner's assertion** for this binding and adapter/model revision.
  They confer no control, edit, capture or execution permission, and they do not make the owner
  authentic: the descriptor digest and catalog digest are integrity markers, and a caller crossing
  an owner boundary must still pin the expected owner/adapter/model revisions out of band.
- Publishing the selected `max_control_events` does not yet enforce it in the control-transition
  path, and the render path still has no production caller for `ContextEffectiveLimits::render_limits()`.
  Both remain outstanding for #95, as do the saved-policy store with adoption history and the
  Console/Studio journeys.
- Native/provider/deployment acceptance is not claimed: these checks are deterministic, synthetic
  and offline.
