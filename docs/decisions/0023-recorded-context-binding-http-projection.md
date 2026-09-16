# ADR 0023: Recorded context-binding HTTP projection

## Status

Accepted for scoped implementation of Harness #100. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

ADR 0040 introduced the private `ascension.harness.context-binding-history.v1` record and a
library-only historical read (`ManagementService::recorded_context_binding`). It deliberately
left the HTTP projection undefined and required any such projection to be separately versioned
rather than silently inferring an owner/consumer contract.

Downstream composition (Context Console #18/#19) needs a bounded, authenticated way to read the
owner response accepted for one workflow invocation without gaining current control authority or
any content bytes.

## Decision

Add one read-only management route:

```
GET /v1/workflow-runs/{run_id}/executions/{node_execution_id}/context-binding
```

The response body is the versioned projection
`ascension.harness.recorded-context-binding-view.v1` (`RecordedContextBindingView`), containing
`schema_version`, `command_id`, `run_revision` and the recorded `binding`.

Authorization and failure behavior are inherited unchanged from the library read:

- current scoped `workflow:read` for the run prefix is required;
- the caller's subject must equal the original binding command's subject;
- disabled or unsupported retention is `503 context_history_unavailable`;
- an invocation with no recorded binding is `404 context_binding_not_recorded`;
- a different subject is `403 context_history_subject`.

The projection is observation-only. It performs no owner lookup, rebind, epoch refresh, provider
call or continuation. Projected grants, epochs and continuity flags are facts of the original
response and never current capabilities. The originating subject is intentionally not projected,
and no content, prompt, provider payload or credential is included. The existing
`GET /v1/workflow-runs/{run_id}/context` current-cursor association is unchanged and remains the
only "current" association surface.

## Compatibility

Additive and read-only. No existing route, request/response schema, private record, SQLite table,
digest or resource ceiling changes. Unknown subpaths and methods continue to return
`route_not_found`, so no previously unknown path becomes ambiguous. Retention stays opt-in and is
rejected by stores that cannot commit history atomically.

## Consequences and limits

- Reading history is not current control authority, receipt recovery, or evidence of a live owner,
  fresh epoch, provider execution or native game state.
- Receipt recovery for lost owner replies and Console/Studio consumer adoption remain outstanding;
  #100 stays open.
- The 16,384-byte per-record and 256-records-per-run bounds from ADR 0040 continue to apply, and
  the management response-size bound still caps the projection.
