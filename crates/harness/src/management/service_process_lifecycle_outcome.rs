// SPDX-License-Identifier: MIT

//! Outcome settlement for the served-live lifecycle command surface.
//!
//! Settling is separated from submission only to keep each module inside the
//! repository size bound; the rule it encodes is unchanged. An answer is
//! validated against the submitted operation, a lost answer is retained as
//! `Unknown` with durable intent rather than lost, and only an unambiguous
//! typed refusal is surfaced as a rejection the caller can rely on.

use crate::management::ManagementError;
use crate::management::ManagementService;
use crate::management::contract::ErrorClass;
use crate::management::lifecycle::{
    LifecycleClassification, LifecycleCommandResponse, LifecycleOperationView, LifecycleState,
    LifecycleTarget, PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION,
};

impl ManagementService {
    /// Records the settled outcome of one lifecycle submission or lookup.
    ///
    /// A transport or gateway unavailability is retained as `Unknown` with the
    /// intent durable, so it is reconcilable rather than lost. Every other
    /// typed refusal is retained as `Rejected` and surfaced to the caller.
    pub(super) fn settle_lifecycle(
        &self,
        command_id: &str,
        run_id: &str,
        operation_id: u64,
        action_kind: &str,
        target: &LifecycleTarget,
        outcome: Result<LifecycleOperationView, ManagementError>,
    ) -> Result<LifecycleCommandResponse, ManagementError> {
        let intents = self.lifecycle_intents.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "process_lifecycle_owner_unavailable",
                "no durable lifecycle intent owner is attached to this composition",
            )
        })?;
        match outcome {
            Ok(view) => {
                view.validate(operation_id, &target.instance_id)?;
                let classification = classify(&view, action_kind);
                let (intent, run_revision) = {
                    let mut intents = intents.lock().map_err(lock_error)?;
                    let intent = intents.complete(
                        &target.instance_id,
                        operation_id,
                        classification,
                        view.operation_state,
                        view.state,
                        view.failure.clone(),
                    )?;
                    drop(intents);
                    let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
                        ManagementError::invalid("run_not_found", "workflow run was not found")
                    })?;
                    (intent, snapshot.run_revision)
                };
                Ok(LifecycleCommandResponse {
                    schema_version: PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION.to_owned(),
                    command_id: command_id.to_owned(),
                    workflow_run_id: run_id.to_owned(),
                    operation_id: view.operation_id,
                    instance_id: view.instance_id,
                    run_revision,
                    classification,
                    operation_state: view.operation_state,
                    state: view.state,
                    authority_epoch: view.authority_epoch,
                    reason_code: format!("lifecycle_{}", classification.as_str()),
                    gameplay_ready: false,
                    failure: intent.failure,
                })
            }
            Err(error) if is_ambiguous(&error) => {
                let mut intents = intents.lock().map_err(lock_error)?;
                intents.complete(
                    &target.instance_id,
                    operation_id,
                    LifecycleClassification::Unknown,
                    crate::management::lifecycle::LifecycleOperationState::Unknown,
                    LifecycleState::Unknown,
                    Some(crate::management::lifecycle::LifecycleFailure {
                        code: error.code.clone(),
                        detail: None,
                    }),
                )?;
                Err(ManagementError::unresolved(
                    "process_lifecycle_outcome_unknown",
                    format!(
                        "lifecycle operation {} may have been applied; reconcile it by identity",
                        operation_id
                    ),
                ))
            }
            Err(error) => {
                let mut intents = intents.lock().map_err(lock_error)?;
                intents.complete(
                    &target.instance_id,
                    operation_id,
                    LifecycleClassification::Rejected,
                    crate::management::lifecycle::LifecycleOperationState::Rejected,
                    LifecycleState::Unknown,
                    Some(crate::management::lifecycle::LifecycleFailure {
                        code: error.code.clone(),
                        detail: None,
                    }),
                )?;
                Err(error)
            }
        }
    }
}

/// Maps one authoritative answer onto the durable classification.
fn classify(view: &LifecycleOperationView, action_kind: &str) -> LifecycleClassification {
    use crate::management::lifecycle::LifecycleOperationState as State;
    match view.operation_state {
        State::Rejected => LifecycleClassification::Rejected,
        State::Stopped if action_kind == "stop" => LifecycleClassification::Stopped,
        State::Unknown | State::Blocked => LifecycleClassification::Unknown,
        State::IntentRecorded
        | State::Starting
        | State::Started
        | State::Attached
        | State::Stopping
        | State::Restarting
        | State::Stopped
        | State::Failed => LifecycleClassification::Accepted,
    }
}

/// True when a failure means the effect may have happened.
///
/// Unavailable and unresolved failures are ambiguous: the gateway may have
/// applied the operation before the answer was lost. Conflicts, refusals, and
/// validation failures are unambiguous and provably effect-free, because the
/// gateway rejects them before any process call.
fn is_ambiguous(error: &ManagementError) -> bool {
    matches!(
        error.class,
        ErrorClass::Unavailable | ErrorClass::Unresolved
    ) || error.code == "process_lifecycle_outcome_unknown"
}

pub(super) fn lock_error<T>(_: std::sync::PoisonError<T>) -> ManagementError {
    ManagementError::unavailable(
        "lifecycle_intent_lock_poisoned",
        "durable lifecycle intent store is unavailable",
    )
}
