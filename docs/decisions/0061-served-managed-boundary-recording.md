# ADR 0061: A served managed decision records its exact boundary before it writes

Status: accepted for the served managed render boundary in `crates/harness`. This record does not
authorize a provider, native-host, game, deployment or paid-call lane, and it changes nothing about
which provider a managed decision reaches or how many times it reaches one. It bounds what a served
composition may *claim* about the bytes it wrote.

## Context

`context_capture` owns approval, holding, fence drift, cancellation, receipts and once-only release
for prepared application input (`PreparedApplicationInput`, `PreparedDispatchController`,
`PreparedDispatchPort`, `DispatchLedger`, `ADVERTISED_EXACT_ADAPTERS`). None of it had a production
caller: the served managed decision rendered a `PreparedContext` and handed its bytes straight to
`decide_prepared_for`, so the material the renderer produced and the bytes the provider received were
two values that agreed only by construction. The claim "the provider received exactly what was
approved" therefore had no observation behind it — the same process both produced the value and
asserted the agreement.

The library could not close that gap on its own. Approval, fences and receipts exist to be *driven*
by the code that owns the boundary, and a boundary owner that passes the renderer's output to the
transport without recording it cannot be corrected from below.

## Decision

The served managed decision drives the held prepared-dispatch protocol, and the provider exchange
moves inside the recording write port, so the approved material and the written bytes are one value.

- The single recorded component is the encoded request the harness itself writes to the Exo transport
  at `adapter.cli_input` (`CaptureBoundary::ExoSessionRequest`), kind `Stdin`, ordinal `0`, media type
  `application/json`. The claim is deliberately narrow: provider-internal conversation, hidden context
  and the effective provider window are never claimed, and an adapter that advertises no exact
  boundary can never be prepared.
- The dispatch identity is derived from the served execution identity
  (`exo.<first 32 hex of sha256(execution_id)>`) rather than a per-process counter, so a repeated
  release of the same invocation resolves to the approval it already wrote.
- `DispatchFences` binds every axis this composition can observe, each non-digest value through a
  domain-separated, length-prefixed digest so a change in any contributing value makes a held
  approval stale. `revocation_epoch` has no served source and is recorded as a residual rather than
  claimed as coverage.
- The write port is attached to the served session as `BoundaryCaptureSink`. Its default cannot
  record, and a release whose sink cannot record is refused with the capability error
  `prepared_boundary_unsupported` *before* the provider boundary, so exactness is never published for
  a boundary nothing recorded. The served composition attaches `BoundaryCaptureSink::memory_ring()`,
  a bounded in-memory recording ring, so a served managed decision records its boundary instead of
  refusing; a composition that attaches no sink (the inert default) still refuses.
- A refusal reported before the exchange stays a clean refusal. Anything that may already have
  reached the provider is reported as `Unresolved`, and a retained receipt is authoritative: a
  repeated release returns it and writes nothing a second time.

## Consequences

The served claim now rests on an observation the boundary owner made. `crates/harness` acceptance
tests drive the real `decide_for` path and assert the recorded bytes against the bytes the transport
received, the refusal without a sink, the stale-source fence before any write, the once-only release,
and the absence of an exactness claim for an unadvertised adapter; removing the recording port makes
the first of them fail rather than merely reduce coverage.

The cost is a recording obligation: a composition that enables managed rendering without attaching a
sink now refuses instead of serving. That is the intended direction — refusing is the safe outcome
when the alternative is publishing an exactness claim nothing observed. The served composition
satisfies it with a bounded in-memory ring, which drops its oldest record once full.

Honest limits of this record:

- The served composition records through a bounded in-memory ring, so the recorded bytes do not
  survive a process restart. The receipt ledger that backs the once-only release has a versioned
  durable image, an owner-supplied reconciliation port (`with_dispatch_ledger_port`) and a
  file-backed store, and the served binary attaches that store when `STS2_WORKFLOW_DISPATCH_LEDGER`
  names an image path. The default stays session-lifetime, so which store a deployment commits its
  receipts to remains an owner decision.
- Only the Exo lane is recorded. The Ollama `HttpBody` boundary is advertised by the library but this
  served composition does not record it.
- Live Exo connectivity, native-host progression, gameplay outcomes and any paid provider exchange
  remain unverified here; every fixture is synthetic and application-controlled.
- A durable decision-level reconciliation port (see [ADR 0059](0059-held-live-decision-attempt.md))
  now exists: a composition that attaches one reloads the committed receipts and refuses a second
  write after a restart. The served composition attaches none unless the operator sets
  `STS2_WORKFLOW_DISPATCH_LEDGER`, so without one the once-only guarantee still rests on the
  in-memory receipt ledger of one served session plus the caller's held-attempt discipline.
