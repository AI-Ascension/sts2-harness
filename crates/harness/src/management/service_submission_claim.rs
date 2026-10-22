// SPDX-License-Identifier: MIT

use super::super::contract::{
    SUBMISSION_SCHEMA_VERSION, SubmissionClaim, SubmissionClaimAcceptance, SubmissionClaimOutcome,
    SubmissionRecoveryResponse, SubmissionRecoveryState,
};
use super::support::{authorize, recovery_admission, run_submission_response};
use super::target_admission::is_live_profile;
use super::*;

impl ManagementService {
    pub(super) fn submit_with_claim(
        &self,
        actor: &AuthContext,
        request: RunRequest,
        request_digest: String,
    ) -> Result<RunSubmissionResponse, ManagementError> {
        let request_id = request.request_id.clone();
        let mut claim = SubmissionClaim {
            actor_digest: actor_digest(actor)?,
            request_digest: request_digest.clone(),
            outcome: None,
        };
        match self.store.claim_submission(&request_id, claim.clone())? {
            SubmissionClaimAcceptance::Acquired => {}
            SubmissionClaimAcceptance::Existing(existing) => {
                return self.replay_claim(actor, &request, &request_digest, existing);
            }
            SubmissionClaimAcceptance::Conflict => {
                return Err(ManagementError::conflict(
                    "submission_conflict",
                    "request_id belongs to a different or legacy submission",
                ));
            }
        }
        // No external port is invoked before the store commits the actor/request fence.
        let result = self.submit_claimed_run(actor, request, request_digest.clone());
        claim.outcome = Some(match &result {
            Ok(response) => SubmissionClaimOutcome::Submitted {
                response: response.clone(),
            },
            Err(error) => {
                let workflow_run_id =
                    match self.store.lookup_submission(&request_id, &request_digest) {
                        Ok(SubmissionLookup::Existing(snapshot)) => Some(snapshot.workflow_run_id),
                        Ok(SubmissionLookup::Missing) => None,
                        _ => return Err(persistence_unknown()),
                    };
                SubmissionClaimOutcome::Failed {
                    workflow_run_id,
                    error: error.error_response().error,
                }
            }
        });
        self.store
            .finish_submission_claim(&request_id, claim)
            .map_err(|_| persistence_unknown())?;
        result
    }

    fn replay_claim(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        request_digest: &str,
        claim: SubmissionClaim,
    ) -> Result<RunSubmissionResponse, ManagementError> {
        match claim.outcome {
            Some(SubmissionClaimOutcome::Submitted { response }) => {
                match self
                    .store
                    .lookup_submission(&request.request_id, request_digest)?
                {
                    SubmissionLookup::Existing(snapshot)
                        if snapshot.workflow_run_id == response.workflow_run_id =>
                    {
                        self.recover_existing_run(actor, request, &snapshot)
                    }
                    _ => Err(persistence_unknown()),
                }
            }
            Some(SubmissionClaimOutcome::Failed {
                workflow_run_id: Some(run_id),
                ..
            }) => {
                authorize(actor, "workflow:control", Some(&run_id))?;
                Err(ManagementError::write_state_unknown(
                    "live_submission_recovery_required",
                    "a failed reserved submission requires recovery of the original request_id",
                ))
            }
            Some(SubmissionClaimOutcome::Failed { error, .. }) => {
                Err(ManagementError::new(error.class, error.code, error.message))
            }
            None => {
                if let SubmissionLookup::Existing(snapshot) = self
                    .store
                    .lookup_submission(&request.request_id, request_digest)?
                {
                    authorize(actor, "workflow:control", Some(&snapshot.workflow_run_id))?;
                }
                Err(ManagementError::write_state_unknown(
                    "submission_pending",
                    "recover the original request_id; submission may still finish and must not be repeated",
                ))
            }
        }
    }

    pub(super) fn recover_existing_run(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        snapshot: &RunSnapshot,
    ) -> Result<RunSubmissionResponse, ManagementError> {
        authorize(actor, "workflow:control", Some(&snapshot.workflow_run_id))?;
        if is_live_profile(&request.profile) {
            let recovery = self
                .execution
                .recovery_admission(snapshot)
                .unwrap_or_else(|| recovery_admission(snapshot));
            if matches!(
                recovery,
                RecoveryAdmission::NeedsOperator | RecoveryAdmission::Reconciling
            ) {
                return Err(ManagementError::unresolved(
                    "live_submission_recovery_required",
                    "a reserved live submission requires reconciliation before retry",
                ));
            }
        }
        Ok(run_submission_response(snapshot))
    }

    /// Recover a submission by its original identity without running admission or launch again.
    /// Foreign and missing identities have the same response; no payload digest is disclosed.
    pub fn submission_recovery(
        &self,
        actor: &AuthContext,
        request_id: &str,
    ) -> Result<SubmissionRecoveryResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        validate_identifier("request_id", request_id)?;
        let mut response = SubmissionRecoveryResponse {
            schema_version: SUBMISSION_SCHEMA_VERSION.to_owned(),
            request_id: request_id.to_owned(),
            state: SubmissionRecoveryState::Missing,
            workflow_run_id: None,
            response: None,
            error: None,
        };
        let Some(claim) = self.store.submission_claim(request_id)? else {
            return Ok(response);
        };
        if claim.actor_digest != actor_digest(actor)? {
            return Ok(response);
        }
        match claim.outcome {
            None => {
                response.state = SubmissionRecoveryState::Pending;
                if let SubmissionLookup::Existing(snapshot) = self
                    .store
                    .lookup_submission(request_id, &claim.request_digest)?
                {
                    response.workflow_run_id = Some(snapshot.workflow_run_id);
                }
            }
            Some(SubmissionClaimOutcome::Submitted {
                response: submitted,
            }) => {
                response.state = SubmissionRecoveryState::Submitted;
                response.workflow_run_id = Some(submitted.workflow_run_id.clone());
                response.response = Some(submitted);
            }
            Some(SubmissionClaimOutcome::Failed {
                workflow_run_id,
                error,
            }) => {
                response.state = SubmissionRecoveryState::Failed;
                response.workflow_run_id = workflow_run_id;
                response.error = Some(error);
            }
        }
        if let Some(run_id) = &response.workflow_run_id {
            authorize(actor, "workflow:read", Some(run_id))?;
        }
        Ok(response)
    }
}

fn actor_digest(actor: &AuthContext) -> Result<String, ManagementError> {
    Ok(digest_value(&json!({ "subject": actor.subject }))?)
}

fn persistence_unknown() -> ManagementError {
    ManagementError::write_state_unknown(
        "submission_outcome_persistence_failed",
        "submission outcome persistence failed; recover the original request_id and do not relaunch",
    )
}
