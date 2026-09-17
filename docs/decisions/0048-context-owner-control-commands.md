# ADR 0048: Context-Owner Control Commands over Management HTTP

## Status

Accepted for the Harness management boundary. This record does not authorize a
provider, native-host, game, deployment, or paid-call lane, and it does not
change the context owner's authority over its own control state.

## Context

The management surface already exposed the context owner's current association
([ADR 0025](0025-context-owner-current-association.md)), its composed effective
limits ([ADR 0030](0030-context-owner-effective-limits-composition.md)), recovery
of already recorded receipts ([ADR 0024](0024-context-control-receipt-recovery.md))
and one commit path, source adoption
([ADR 0044](0044-served-managed-context-source.md)). `ContextOwnerPort::control`
existed with no HTTP caller, so a downstream composition (Context Console #18,
Studio) could observe the owner but could not ask it to pause, commit an
approved manifest, or resume. Separately, `EnvironmentAuthenticator` minted one
`workflow:*` token per profile, so a served process could not demonstrate that a
metadata-only caller is refused by the harness scope guards.

## Decision

`POST /v1/workflow-runs/{run_id}/context-control-commands` accepts one
`ContextControlCommand` (`pause`, `commit` or `resume`, each carrying its
idempotency key and its pre-command fence) under scoped `workflow:control` and
returns the owner-issued `ascension.context-control.owner-receipt.v2`.

The route **submits** the command to the authoritative owner; it never mints
control authority itself. Before the owner is called it:

1. composes the run's current binding through the same fail-closed seam the
   effective-limits projection and the control-authority binding use, so an
   unattached owner, a foreign or non-published binding, or a tampered catalog
   is refused first;
2. returns the owner's already recorded receipt when the exact command (same
   idempotency key and payload) was recorded before, with the identity checks
   of ADR 0024, so a retried request cannot apply a second effect;
3. refuses a stale pre-command fence with a typed conflict and no receipt:
   `context_control_fence_stale` (control version), `context_control_boundary_stale`
   (the `expected_boundary` of a commit or resume) and
   `context_control_revision_stale` (a commit's `expected_revision_id`).

After the owner answers, the receipt is validated against the binding and the
command exactly as a recovered receipt is, so an owner answering for a different
transition is refused rather than projected. The owner remains the authority for
its own state machine: pause readiness, idempotency-key reuse with a different
payload, event-limit exhaustion and lease or actor staleness are its refusals
and pass through unchanged (`context_owner_control_refused`,
`context_control_events_exhausted`, `context_owner_control_stale`). Grants
projected in the binding stay owner assertions, as in ADR 0025.

The same profile may now mint a read-only companion credential.
`STS2_WORKFLOW_TOKEN_<PROFILE>_READ`, when set, authenticates to the **same**
profile subject with `workflow:read` only, so the served owner's association
and receipt reads remain reachable while content writes, adoption and control
are refused with `missing_scope`. The companion must differ from the primary
token. `EnvironmentAuthenticator::from_profile_with` takes an explicit variable
lookup so compositions and tests can supply tokens without touching the process
environment; `from_profile` keeps reading the environment.

## Compatibility

`additive-compatible`. One route and one optional environment variable are
added. No existing route, field, durable record, schema, digest, bound or
default changes; `POST /v1/workflow-runs/{run_id}/context-control-receipts/lookup`
keeps its behaviour and now shares the recorded-receipt helper with submission.
A profile without the companion variable mints exactly the credential it minted
before. The v2 receipt schema is unchanged, so consumers pinned to ADR 0024
read the submission response without change.

## Consequences and limits

Evidence for this record is in-process: a synthetic owner behind the management
HTTP server, an in-memory store, and no provider or game. The served
process-restart case (`served_workflow_recovers_context_receipt_after_process_restart`)
still records a single adoption commit; extending it to a pause, commit, resume
sequence is a separate operator-gated change. Native, provider and deployment
behaviour remain unverified here.
