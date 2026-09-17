# ADR 0025: Current context-owner association over HTTP

## Status

Accepted for scoped implementation of Harness #100. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

Two different "current context" surfaces already existed but only one was reachable:

- `GET /v1/workflow-runs/{run_id}/context` returns `ContextAssociation`, composed from the injected
  `ContextInspectionPort` — the harness-side view of the current cursor, prepared context and
  capability disclosure; and
- `ContextOwnerPort::association(actor, snapshot)` returns the **authoritative owner's** current
  `ContextOwnerBinding` for a run. It was only reachable internally, from the receipt-recovery path
  added in ADR 0024.

Downstream composition (Context Console #18/#19, Studio) needs to read the owner's authoritative
current binding — which owner, which invocation, which binding identity and version, which state,
and which epochs — without gaining control authority or content bytes. Neither the inspection-port
association nor the historical recorded-binding projection (ADR 0023) provides that.

## Decision

Expose the authoritative current association as a separately versioned, read-only projection:

```
GET /v1/workflow-runs/{run_id}/context-owner-association
```

The body is `ascension.harness.context-owner-association-view.v1`
(`ContextOwnerAssociationView { schema_version, binding }`), where `binding` is the owner's current
`ContextOwnerBinding` for that run.

Behavior:

- current scoped `workflow:read` for the run prefix is required;
- an unknown run is `400 run_not_found`;
- an unattached, unavailable or refusing owner stays explicitly unavailable
  (`503 context_owner_association_unavailable` from the default port), never "no association";
- a binding whose `workflow_run_id` does not match the requested run fails closed with
  `409 context_binding_mismatch` rather than being returned.

The binding is resolved through one shared helper (`ManagementService::current_context_binding`) for
the current-association and live-control paths. Historical receipt recovery uses a separate
persisted-evidence path and must not fabricate a current association after restart.

## Compatibility

Additive and read-only. No existing route, request/response schema, private record, SQLite table,
digest or resource ceiling changes. `GET /v1/workflow-runs/{run_id}/context` keeps its existing
`ContextAssociation` contract and semantics untouched; the two surfaces are deliberately distinct
because one is harness-inspected metadata and the other is an owner assertion. Unknown paths and
methods still fail closed with `route_not_found`.

## Consequences and limits

- The projected grants, epochs and continuity flags are the **owner's assertions about the current
  binding**. They are not harness-issued authority: this route confers no control, edit, capture or
  execution permission and cannot be used to authorize a mutation.
- A current association is not evidence of provider execution, native game state or an accepted
  control effect; those remain separately gated.
- Console/Studio adoption, provider-session saved-policy work and the remaining #100 journeys stay
  outstanding; #100 remains open.
