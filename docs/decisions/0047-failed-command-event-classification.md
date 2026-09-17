# ADR 0047: Failed-Command Event Classification

## Status

Accepted for the Harness management event contract. This record does not
authorize a provider, native-host, game, deployment, or paid-call lane.

## Decision

A live command that faults before it executes anything is journalled as
`command_applied` with `classification: rejected`, not `settled`.

The generic runtime-fault arm of `apply_command` returns
`CommandOutcome::Applied` with reason code `live_execution_failed` while the run
fails and the cursor stays on the same node. The response outcome is unchanged:
the command *was* processed. But `classification_for_outcome` mapped every
`Applied` response to `EventClassification::Settled`, so a failure that executed
nothing looked like forward progress to any consumer that treats a settled step
as completed, and to recovery logic keyed on settled-but-failed transitions.

Classification is now derived from the outcome **and** the reason code in one
place (`CommandOutcome::classification`), used by both the in-memory and SQLite
event writers. The single failing reason code classifies as `Rejected`.

## Why the outcome vocabulary is unchanged

The issue proposed either a new `CommandOutcome` variant or a reason-aware
classification. Adding a variant would extend the closed `ascension.management/v1`
outcome set. The pinned consumer schemas are closed (`additionalProperties: false`
with enumerated values), and this repository's compatibility policy requires
explicit version/capability negotiation plus coordinated producer/consumer pin
updates before extending a closed published schema. `ascension.management/v1`
carries no such negotiation for this change.

The event classification enum already publishes `rejected`
(`ascension.workflow-event/v1`), so the truthful distinction is available with no
schema change, no new identifier, and no consumer migration. It also removes a
duplicated classifier: the same mapping previously existed in two modules, which
is how one could drift from the other.

## Invariant

A command that fails without executing anything must not be journalled with
`classification: settled`. `live_execution_failed` is the reason code that
identifies that case; the regression test drives a `decide` node that faults
before any provider call and asserts the event carries `rejected` while the run
is failed, no provider call is consumed, and the cursor does not advance.

## Compatibility

This is a `safety-correction` to an unreleased candidate: it changes the
`classification` value emitted for one failure path and adds no field, route,
record, or schema version. Both classifications are already published in the
closed event enum. Existing callers that treat `settled` as completion now see a
refusal instead of a false advance. Nothing depends on the previous value in
this repository.

Evidence is synthetic: the live workflow test double faults the node before any
provider call. No native gameplay, real provider, or served-bytes claim is made.
Refs #260.
