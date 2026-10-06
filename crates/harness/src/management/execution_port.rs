// SPDX-License-Identifier: MIT

use super::super::super::auth::AuthContext;
use super::super::super::contract::{
    PendingOperation, RecoveryAdmission, RunRequest, RunSnapshot, TargetAdmissionBinding,
};
use super::super::super::service::{
    CommandApplication, CommandContext, ManagementError, RunAdmission, RunReservation,
    WorkflowExecutionPort,
};
use super::LiveWorkflowExecutionPort;

impl WorkflowExecutionPort for LiveWorkflowExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        Err(Self::unreserved_submission_error())
    }

    fn submit_admitted(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
        _admission: Option<&TargetAdmissionBinding>,
    ) -> Result<RunAdmission, ManagementError> {
        Err(Self::unreserved_submission_error())
    }

    fn submit_admitted_with_reservation(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
        reserve: &RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        let admission = admission.ok_or_else(|| {
            ManagementError::conflict(
                "target_admission_required",
                "live execution requires an exact target admission binding",
            )
        })?;
        if request.admission.as_ref() != Some(admission) {
            return Err(ManagementError::conflict(
                "target_admission_mismatch",
                "execution admission does not match the submitted request",
            ));
        }
        self.submit_inner(request, actor, definition_digest, reserve, false)
    }

    fn prepare_seed_candidate_with_reservation(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
        reserve: &RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        let admission = admission.ok_or_else(|| {
            ManagementError::conflict(
                "target_admission_required",
                "live seed candidate requires an exact target admission binding",
            )
        })?;
        if !reserve.is_seed_candidate() {
            return Err(ManagementError::conflict(
                "seed_candidate_reservation_missing",
                "seed candidate path requires its service-owned seed reservation",
            ));
        }
        if request.admission.as_ref() != Some(admission) {
            return Err(ManagementError::conflict(
                "target_admission_mismatch",
                "execution admission does not match the submitted request",
            ));
        }
        self.submit_inner(request, actor, definition_digest, reserve, true)
    }

    fn apply_command(
        &self,
        context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        super::super::execution_commands::apply_command(self, context, None)
    }

    fn apply_command_with_intent(
        &self,
        context: CommandContext,
        record_intent: &dyn Fn(PendingOperation) -> Result<(), ManagementError>,
    ) -> Result<CommandApplication, ManagementError> {
        super::super::execution_commands::apply_command(self, context, Some(record_intent))
    }

    fn recovery_admission(&self, snapshot: &RunSnapshot) -> Option<RecoveryAdmission> {
        super::recovery::admission(self, snapshot)
    }

    fn abort_submission(&self, run_id: &str) -> Result<(), ManagementError> {
        super::recovery::abort(self, run_id)
    }
}
