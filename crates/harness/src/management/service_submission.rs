// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

use super::super::auth::AuthContext;
use super::super::contract::{
    CleanupState, RunRequest, RunSnapshot, RunSubmissionResponse, WorkflowRunStatus,
};
use super::support::{run_submission_response, verify_admission};
use super::target_admission::{bind_snapshot_admission, is_live_profile};
use super::{ManagementError, ManagementService, RunAdmission};

type ReservationState = Arc<Mutex<Option<RunSnapshot>>>;

pub(super) fn submit_run(
    service: &ManagementService,
    actor: &AuthContext,
    request: RunRequest,
    request_digest: String,
    definition_digest: String,
) -> Result<RunSubmissionResponse, ManagementError> {
    let binding = service.revalidate_target_admission(actor, &request, &definition_digest)?;
    let reservation_binding = binding.clone();
    let reservation_store = Arc::clone(&service.store);
    let reservation_request_id = request.request_id.clone();
    let reservation_request_digest = request_digest.clone();
    let reservation_definition_digest = definition_digest.clone();
    let reservation_state: ReservationState = Arc::new(Mutex::new(None));
    let reservation_state_for_closure = Arc::clone(&reservation_state);
    let reserve = move |candidate: &RunAdmission| {
        let mut durable = candidate.clone();
        durable.snapshot = bind_snapshot_admission(durable.snapshot, reservation_binding.as_ref())?;
        verify_admission(&durable, &reservation_definition_digest)?;
        let persisted_snapshot = durable.snapshot.clone();
        reservation_store.create_run(
            &reservation_request_id,
            &reservation_request_digest,
            durable.snapshot,
            durable.initial_events,
        )?;
        *reservation_state_for_closure.lock().map_err(|_| {
            ManagementError::store(
                "reservation_state_lock",
                "live reservation state lock is poisoned",
            )
        })? = Some(persisted_snapshot);
        Ok(())
    };
    let execution_result = service.execution.submit_admitted_with_reservation(
        &request,
        actor,
        &definition_digest,
        binding.as_ref(),
        &reserve,
    );
    let mut admission = match execution_result {
        Ok(admission) => admission,
        Err(error) => {
            return Err(persist_reserved_failure(
                service,
                &reservation_state,
                &request,
                &request_digest,
                error,
            ));
        }
    };
    let bound_snapshot = match bind_snapshot_admission(admission.snapshot.clone(), binding.as_ref())
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(persist_reserved_failure(
                service,
                &reservation_state,
                &request,
                &request_digest,
                error,
            ));
        }
    };
    admission.snapshot = bound_snapshot;
    if let Err(error) = verify_admission(&admission, &definition_digest) {
        return Err(persist_reserved_failure(
            service,
            &reservation_state,
            &request,
            &request_digest,
            error,
        ));
    }
    if is_live_profile(&request.profile)
        && let Err(error) = service.store.update_run_snapshot(
            &request.request_id,
            &request_digest,
            admission.snapshot.clone(),
        )
    {
        return Err(persist_reserved_failure(
            service,
            &reservation_state,
            &request,
            &request_digest,
            error.into(),
        ));
    }
    Ok(run_submission_response(&admission.snapshot))
}

fn persist_reserved_failure(
    service: &ManagementService,
    state: &ReservationState,
    request: &RunRequest,
    request_digest: &str,
    original: ManagementError,
) -> ManagementError {
    let snapshot = match state.lock() {
        Ok(mut state) => state.take(),
        Err(_) => {
            return ManagementError::store(
                "reservation_failure_persist",
                format!(
                    "submission failed ({}), and its reservation state could not be read",
                    original.code
                ),
            );
        }
    };
    let Some(mut snapshot) = snapshot else {
        return original;
    };
    snapshot.status = WorkflowRunStatus::NeedsOperator;
    snapshot.cleanup = CleanupState::NeedsOperator;
    match service
        .store
        .update_run_snapshot(&request.request_id, request_digest, snapshot)
    {
        Ok(()) => original,
        Err(persist_error) => ManagementError::store(
            "reservation_failure_persist",
            format!(
                "submission failed ({}), and its reservation could not be marked for recovery ({})",
                original.code, persist_error.code
            ),
        ),
    }
}
