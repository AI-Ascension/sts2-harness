# ADR 0024: Recovered context-control receipts

## Status

Accepted for scoped implementation of Harness #100. No consumer adoption, production owner
composition or feature closure is approved here.

## Context

The context-owner contract already carries two facts that nothing consumed:

- `ContextBindingContinuity::receipt_recovery`, which states whether the authoritative owner can
  return the receipt it already issued for a previously accepted control command; and
- `ContextControlReceipt` (v2), which encodes the exact owner invocation, binding, command variant,
  idempotency key and resulting boundary of one accepted `pause`/`commit`/`resume` effect.

When a delegated control reply is lost or ambiguous, a caller could not distinguish "the owner never
accepted this command" from "the owner accepted it and the reply was lost". Re-issuing the command
would risk a repeated effect; assuming failure would misreport an applied transition.

`ContextOwnerPort` had no lookup method, so receipt recovery was not implementable at all.

## Decision

Define receipt recovery as a read-only lookup, never a re-issue:

1. `ContextOwnerPort::control_receipt(actor, binding, command)` returns
   `Ok(Some(receipt))` for a recorded command, `Ok(None)` when the owner has no receipt for that exact
   command, and `Err` for owner failures. The default implementation fails closed with
   `503 context_owner_receipt_recovery_unavailable`, so existing owners are unaffected (additive).
2. `ManagementService::recover_context_control_receipt(actor, run_id, command)` requires current scoped
   `workflow:read` for the run, resolves the owner's **current** association for that run, and refuses
   to ask an owner that does not advertise `receipt_recovery`
   (`503 context_control_receipt_recovery_unsupported`).
3. A recovered receipt must satisfy `ContextControlReceipt::validate_for(binding, command)` — exact
   owner, invocation, binding id/digest, command variant, idempotency key, effect and resulting
   boundary. A receipt for another invocation or command is rejected (`409
   context_control_receipt_mismatch` / `context_control_receipt_transition`), and a v1 receipt is
   rejected rather than upgraded (`409 context_control_receipt_schema_unsupported`).
4. `POST /v1/workflow-runs/{run_id}/context-control-receipts/lookup` with the retained command as the
   request body exposes the lookup to authenticated callers. The receipt's own
   `ascension.context-control.owner-receipt.v2` `schema_version` governs the response body; no new
   wrapper schema is introduced. An unrecorded command is `404 context_control_receipt_not_recorded`.

## Compatibility

Additive and read-only. The port method has a failing default, so no existing owner, binding, catalog,
store, receipt or digest changes. No effect is issued, re-applied or inferred, and no current control
authority is conferred: a recovered receipt describes a transition the owner already applied.

## Consequences and limits

- Recovery reports what the owner recorded; it is not evidence of a live owner, a fresh epoch, provider
  execution or native game state, and it does not authorize further control.
- Owners that cannot recover receipts remain explicitly unsupported rather than silently treated as
  "no effect".
- Console/Studio adoption of this lookup, provider-session saved-policy work and the remaining #100
  integration journeys are still outstanding; #100 stays open.
