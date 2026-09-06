# Bounded model action plans

- Status: Accepted; bounded Windows combat verified, broader runtime evidence pending
- Date: 2026-09-06
- Owner: Harness

## Contract

A model response may propose an ordered list of at most eight current legal action IDs.
The harness retains the corresponding semantic action payloads and the originating model
execution identity. It dispatches one action at a time through the existing MCP and gateway
path and requires verified settlement before considering the next planned action.
An acknowledgement, generation change, timeout, or observation alone cannot advance a plan.
Rejected, cancelled, or unresolved actions discard the remaining plan without strategic retry.

Each subsequent action must bind uniquely to the fresh host catalog by its full payload.
Generation-scoped action IDs are never edited or reused across observations. Plans are bounded
to one combat turn or shop visit. Newly drawn or changed cards, changed enemy intent, new or
changed shop offers, phase changes, or an unavailable next action require another model decision.
The harness does not predict hidden outcomes or choose replacement actions.

The existing single-action decision remains supported. The proposed plan response is a harness
provider contract extension; older parsers reject it. Bridge and harness must therefore be
installed together. The game-facing Runtime-v3 wire contract remains unchanged.
Every executed step retains its current observation and original model execution identity so
that action replay does not require recreating provider calls or replaying stale IDs.

## Settlement feedback

`DecisionSource::action_completed` is an additive default callback. The episode runner reports
true only after the selected action increased the verified transition count, including verified
same-operation reconciliation. Failure and rejection report false. Recording wrappers forward
the callback. Existing stateless decision sources can retain the default no-op implementation.

## Required evidence

Parser bounds and catalog membership, fresh semantic rebinding, settlement gating, rejection
and unresolved-outcome invalidation, draw and shop-price interruptions, original model identity,
and replay require deterministic tests. Real Astra combat and shop observations must separately
establish several settled actions from a single provider response. Source tests do not prove
latency improvement, live compatibility, or full campaign completion.

The [dated evidence](../evidence/bounded-astra-plans-20260906.md) records a real Astra response
producing three settled card plays and end turn, plus the implementation's local checks.
