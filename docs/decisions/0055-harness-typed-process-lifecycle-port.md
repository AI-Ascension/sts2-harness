# ADR 0055: Harness Typed Process-Lifecycle Port and Durable Intent

## Status

Accepted for the Harness served-live composition. This record does not
authorize a native OS process or signal adapter, a game-process lane, or a
launch capability in the harness, and it changes no other owner's authority.

## Context

The wire contract this record consumes is the **gateway's** ADR 0035,
`docs/decisions/0035-attached-process-lifecycle-route-surface.md` in
`sts2-gateway`, merged as gateway `afb30ba9`. That is a different repository
and a different record from this repository's ADR 0035. It froze a closed
lifecycle surface:
`POST /v1/instances/{instance}/process-lifecycle/operations`,
`GET /v1/instances/{instance}/process-lifecycle`, and
`GET /v1/instances/{instance}/process-lifecycle/operations/{id}`, with a
submission body carrying only `operation_id`, `authority_epoch` and one
`action`. Before this record the harness had no lifecycle route, no durable
intent and no accepted active-run authority, so a caller could not ask for a
launch without inventing a private path to a process.

Ownership is already fixed elsewhere.
[ADR 0001](0001-harness-ownership-and-dependency-boundary.md) and
`AGENTS.md` give the gateway game-instance lifecycle, allocation, routing,
leases, readiness and fencing, and forbid the harness direct game-process
access. This record decides only how the harness *consumes* that surface.

## Decision

The harness owns the **typed port, the durable intent and the
reconciliation**; the gateway owns the process authority. The harness never
acquires OS or game-process authority by consuming this surface.

1. **Contract identity is the gateway's.** The port speaks
   `sts2-gateway-process-lifecycle-v1` and pinpoints the reviewed gateway
   revision `afb30ba9`. A foreign contract value is refused rather than
   coerced, so a contract drift is visible instead of silently accepted.
2. **The action vocabulary is closed.** A submission names only an opaque
   approved launch profile id, a `graceful`/`force` stop mode, or the exact
   process identity a caller previously received. There is no command, path,
   URL, environment or adoption input in the port, so an arbitrary effect
   cannot be expressed even by a buggy caller, and the harness resolves no
   executable.
3. **The instance is the run's admitted target binding.** A run without that
   binding fails closed. There is no configured instance, no default and no
   fallback, so a wrong-target submission has nothing to fall back to.
4. **Intent precedes effect.** A durable intent record is written *before* the
   gateway is called and is stored beside the existing append-only operation
   journal in the same owner-controlled directory. A journal entry without its
   intent record is treated as corruption rather than reconstructed.
5. **A lost response is not a lost effect.** A transport failure after
   submission is not an error to the caller: the operation stays retained as
   `Unknown` and the next reconciliation reads the gateway's own record by
   operation identity. Reconciliation is identity-addressed, so it never
   guesses the action from similar-looking state.
6. **Answers are re-scoped before they are recorded.** Contract, operation,
   instance and epoch are revalidated against the submitted command, so a
   stale or foreign answer cannot be recorded as this command's outcome. A stop
   intent dominates a restart in the same reconciliation.
7. **Launch is structurally not readiness.** `LaunchAcknowledgement` is
   deliberately not `GameplayReadinessEvidence`: the marker trait is sealed,
   the readiness observation requires its own `observation_id` and
   `observation_digest` from a readiness owner, and neither type converts to
   the other in either direction. A launch success therefore cannot satisfy a
   gameplay readiness predicate.
8. **The harness mirrors the three routes, it does not mint authority.** The
   run-scoped management surface exposes one capability read, one submission
   and one identity-addressed reconciliation read. Identity, instance and
   authority all come from the admitted run and the harness configuration,
   never from the request body, and the port asserts no capability the gateway
   did not advertise.

## Honest limits

- The gateway advertises `available: false` today because no concrete OS
  adapter exists (gateway ADR 0024), so every submission on a real deployment
  is refused until one is accepted. The harness mirrors that advertisement and
  does not create capability.
- The boundary suite drives a **recording synthetic port**, not the gateway's
  OS adapter, so it proves the harness's ordering, identity, retention and
  readiness separation. It is **not** native launch, stop or attach evidence
  (gateway `#50` AC5 remains open).
- Re-scoping an answer proves the harness refuses a foreign-scope answer. It
  does not prove the gateway's own fencing, lease-expiry or stop-dominance
  semantics beyond what gateway ADR 0035 already records.

## Consequences

- The change is **additive-compatible**: no existing route, record, schema,
  digest, range or default changes. Three harness routes and one durable
  intent sidecar are added, and one new table is not required because the
  intent store reuses the existing operation journal.
- A crash between intent and effect leaves inspectable evidence rather than an
  unaccounted effect, and a restart replays and reconciles the retained
  operation instead of resubmitting it.
- A future native adapter remains a separate, per-OS accepted boundary; this
  record neither requires nor permits the harness to own it.

## Verification

`crates/harness/tests/process_lifecycle_http.rs` exercises the real management
HTTP surface with a recording port and covers the four acceptance criteria:
duplicate-submission identity and single launch; wrong
target/profile/epoch/unowned-attach with zero downstream effects; lost
response, blocked stop, cancellation and restart with durable reconcilable
outcomes; and launch acknowledgement failing a readiness predicate.
