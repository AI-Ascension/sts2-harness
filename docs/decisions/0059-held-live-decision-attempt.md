# ADR 0059: A held attempt identity for a live decision exchange

Status: accepted for the authored-workflow live `Decide` node in `crates/harness`. This record does
not authorize a provider, native-host, game or deployment lane, and it changes nothing about when a
provider exchange is made: it bounds how many of them one decision may cost.

## Context

The served live path (`LiveNodeExecutor::decide` -> `LiveWorkflowSession::decide_for`,
`ProductionLiveWorkflowSession`) computed its `ModelExecutionId` as `provider_calls + 1` on every
entry and kept no record of the attempt. The action path beside it had the opposite discipline since
the operation ledger landed: a `PendingDispatch` identity is installed *before* the mutating
boundary, recorded durably through the command's intent recorder, and reconciled by exact identity.

The asymmetry was a spend and safety defect with two faces:

- A `ManagementError` of class `Unresolved` or `Unavailable` — a reply lost on the wire, or a
  provider that was reached and did not answer — was mapped to `RuntimeFault::ExecutorUnavailable`,
  so `execution_commands` marked the run `NeedsOperator` / `live_operation_unknown` and the next
  `Step` re-entered `decide` and paid for a second exchange. Nothing in the harness could prove the
  first exchange had not already produced a decision for the same admitted request.
- Nothing about the in-flight attempt reached the run snapshot. `RunSnapshot::pending_operation` is
  populated from `pending`, and recovery admission only inspects that field, so the run published
  `pending_operation: null` with `authority.recovery: "none"` while a paid exchange was outstanding.

## Decision

A live `Decide` node holds one attempt identity under the same discipline as a dispatch.

- `LiveNodeState::pending_decision` holds a `PendingDecision` (operation id, execution id,
  generation, input digest, state, resolved decision). It is installed, and its `Intent` recorded
  through the caller's intent recorder, *before* `decide_for` is called. If the intent cannot be
  recorded the node refuses with `ExecutorUnavailable` before crossing the provider boundary.
- Exactly one outcome releases the hold: a refusal whose `ErrorClass` is not `Unresolved` or
  `Unavailable`, because a provider owner that reported the refusal before it could write is a
  clean, retryable-by-policy refusal. Every `Unresolved` or `Unavailable` refusal is reported as
  `RuntimeFault::UnknownEffect`, which is the fault the served command path already treats as "an
  effect is unresolved, do not clean up and do not replace it".
- A re-entered node reuses the held execution id and must reproduce the digest of the admitted
  request that attempt was paid for. The digest is a canonical JSON envelope over the execution id,
  observation state id, generation, the legal-action catalog (ids and kinds), objective and hard
  constraints. A retry that does not reproduce it exactly may neither retarget the held identity nor
  mint a new one while the held one is unresolved; it keeps the hold and reports `UnknownEffect`.
- Only a usable decision for that exact admitted request releases the hold and advances the node. If
  consuming an accepted decision faults, the hold survives carrying the accepted attempt, because a
  replacement exchange would be indistinguishable from a second payment.
- The hold is projected into the run snapshot exactly as a `PendingDispatch` is: `application`
  publishes `pending_operation` from the held intent when no dispatch is outstanding, so the served
  snapshot reports `authority.recovery: "pending_effect_visible"` and recovery admission reports
  `Reconciling` rather than leaving the run indistinguishable from a state with nothing outstanding.

## Consequences

The duplicate-exchange path is closed at the served boundary and the outstanding attempt is now
durable and visible. `crates/harness/tests/live_workflow_decision_hold.rs` pins four served
behaviours, and each of them fails when its guard is removed: a lost reply holds the attempt and a
re-issued operator step reaches the provider once in total; a restart admits `Reconciling` and
refuses resubmission instead of looking resumable; a pre-write refusal releases the hold so the run
does not advertise an outstanding attempt; and a catalog that changed between the attempts keeps the
original intent instead of paying for the changed request.

What this record does not claim:

- `provider_calls_consumed` still counts only completed exchanges, so an exchange whose reply was
  lost is evidenced by the held attempt and its durable intent event rather than by the budget
  counter. Under-reporting a paid exchange in the published budget is unchanged by this decision.
- There is no decision-level reconciliation: `reconcile_pending` knows only `PendingDispatch`. A
  held attempt therefore blocks every further exchange until it is reconciled by tooling that does
  not exist yet, and an operator's only served forward path today is to cancel the run. Publishing a
  reconciliation surface that can retire a held decision attempt without replacing the harness's
  refusal, and exposing it to Studio and Console consumers, remain outstanding.
- No consume-side caller was added for the source-only `context_capture::dispatch_*` surface, and no
  native, provider or paid call was made while validating this change; the model-override disclosure
  for this lane is recorded in its pull request and lane record.
