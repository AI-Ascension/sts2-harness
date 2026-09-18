# ADR 0052: Served Provider-Session Effective-Limits Record

## Status

Accepted for the Harness management boundary. This record does not authorize a
provider, native-host, game, deployment, or paid-call lane, and it does not
change how a provider session is admitted or which limits it executes under.

## Context

The machine-readable `ascension.harness.effective-limits.v1` record exists with
two library producers, `NativeCapabilities::effective_limit_record` for the
provider-session surface and `MemoryCapabilities::effective_limit_record` for
the context-memory surface, and consumers (Context Console, Studio) already pin
the record schema. The Console's owner capability sidecar (its ADR 0021) was
fed from copied fixtures because no harness transport served the record: the
existing `GET /v1/workflow-runs/{run_id}/context-owner-effective-limits`
([ADR 0030](0030-context-owner-effective-limits-composition.md)) serves the
context owner's composed limits, a different surface. `serve-workflow` already
holds the `NativeCapabilities` descriptor it admits provider sessions against,
so the missing piece was an authenticated read of the producer's output.

## Decision

`GET /v1/workflow-runs/{run_id}/provider-session-effective-limits` returns, under
scoped `workflow:read`, the record the library producer builds from the served
descriptor. The management service holds that descriptor through
`with_provider_session_capabilities`, which validates it at composition; the
served process passes the same descriptor it hands the production session
factory, so the record describes the ceilings sessions are admitted under.

The route serves a record only when:

1. the run is admitted in this process (`run_not_found` otherwise, as the
   sibling run routes report it);
2. the caller holds run-scoped `workflow:read` (`missing_scope`,
   `run_scope_denied`);
3. the composition holds a served descriptor
   (`provider_session_capabilities_unavailable`, 503, otherwise) — the route
   never answers with the library fixture;
4. the run's **current** context-owner association resolves through the same
   fail-closed seam as ADR 0025/0030 (unattached owner, foreign run and the
   other refusals keep their codes), and its boundary names the served
   descriptor's adapter and model revision
   (`provider_session_capabilities_mismatch`, 409, otherwise). The runtime
   authority binding derives the boundary's revisions from the served
   descriptor, so a mismatch means the record would describe a profile this run
   is not bound to.

The produced record is validated before it is returned. It is metadata only:
classification rows (field, class, ceilings, validator) and the descriptor
digest; no enabled-method list, hardening flags, transport, session content,
credential or policy bytes are projected.

**Context-memory record.** The served workflow composition holds no selected
memory corpus and therefore no `MemoryCapabilities` producer. Rather than
fabricate a record or leave the path as an unknown route,
`GET /v1/workflow-runs/{run_id}/context-memory-effective-limits` performs the
same scope and run checks and then refuses with the typed
`context_memory_record_unavailable` (503). Serving that record requires a
composition that owns a corpus; that is a separate decision, and this route is
where it would attach.

## Compatibility

`additive-compatible`. Two routes and one optional service builder are added.
The record schema, both producers, the `fixtures/effective-limits/` bytes, the
consumer pins and every existing route are unchanged; no bound, schema, digest
or default changes. `serve_live_with_provider_policy_commands_and_context_owner`
gains the descriptor parameter; its only caller is the `serve-workflow` binary. Its three owner
ports are now passed as one `ServedOwnerPorts` value instead of three positional arguments, so the
constructor stays within the argument bound as the required owner set grows and a caller cannot
transpose ports of the same type; the ports themselves and their behaviour are unchanged.

## Consequences and limits

Evidence is in-process: a synthetic owner behind the management HTTP server, an
in-memory store and the library capability fixture with one deliberately
lowered ceiling, so a route that hard-coded the fixture record fails the
byte comparison. The record served by a real `serve-workflow` process against a
real provider profile, and the Console-side sidecar population under configured
descriptor/scope/source/epoch trust, remain unverified here.
