// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::{CleanupState, RunRequest, RunSubmissionResponse, WorkflowRunStatus};
use super::support::{run_submission_response, verify_admission};
use super::target_admission::{bind_snapshot_admission, is_live_profile};
use super::{ManagementError, ManagementService, RunReservation};

pub(super) fn submit_run(
    service: &ManagementService,
    actor: &AuthContext,
    request: RunRequest,
    request_digest: String,
    definition_digest: String,
) -> Result<RunSubmissionResponse, ManagementError> {
    let binding = service.revalidate_target_admission(actor, &request, &definition_digest)?;
    let reservation = RunReservation::new(
        std::sync::Arc::clone(&service.store),
        request.request_id.clone(),
        request_digest.clone(),
        definition_digest.clone(),
        binding.clone(),
    );
    let execution_result = service.execution.submit_admitted_with_reservation(
        &request,
        actor,
        &definition_digest,
        binding.as_ref(),
        &reservation,
    );
    let mut admission = match execution_result {
        Ok(admission) => admission,
        Err(error) => {
            return Err(persist_reserved_failure(
                service,
                &reservation,
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
                &reservation,
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
            &reservation,
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
            &reservation,
            &request,
            &request_digest,
            error.into(),
        ));
    }
    Ok(run_submission_response(&admission.snapshot))
}

fn persist_reserved_failure(
    service: &ManagementService,
    reservation: &RunReservation,
    request: &RunRequest,
    request_digest: &str,
    original: ManagementError,
) -> ManagementError {
    let snapshot = match reservation.take_snapshot() {
        Ok(snapshot) => snapshot,
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
    let cleanup_error = service
        .execution
        .abort_submission(&snapshot.workflow_run_id)
        .err();
    snapshot.status = WorkflowRunStatus::NeedsOperator;
    snapshot.cleanup = CleanupState::NeedsOperator;
    match service
        .store
        .update_run_snapshot(&request.request_id, request_digest, snapshot)
    {
        Ok(()) => cleanup_error.map_or(original.clone(), |error| {
            ManagementError::unavailable(
                "live_submission_cleanup_failed",
                format!(
                    "submission failed ({}), and live cleanup failed ({})",
                    original.code, error.code
                ),
            )
        }),
        Err(persist_error) => ManagementError::store(
            "reservation_failure_persist",
            format!(
                "submission failed ({}), and its reservation could not be marked for recovery ({}{})",
                original.code,
                persist_error.code,
                cleanup_error
                    .as_ref()
                    .map_or(String::new(), |error| format!(
                        ", cleanup failed ({})",
                        error.code
                    ))
            ),
        ),
    }
}
