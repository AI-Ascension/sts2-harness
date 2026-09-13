// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::{
    CommandRequest, PendingOperation, RunEvent, RunRequest, RunSnapshot, TargetAdmissionBinding,
};
use super::ManagementError;

pub trait WorkflowExecutionPort: Send + Sync {
    fn submit(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError>;

    /// Submit a run with the exact binding returned by the actor-scoped
    /// preflight. Adapters that do not need a separate admission boundary
    /// inherit a compatibility implementation which still carries the
    /// binding into the durable snapshot.
    fn submit_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
    ) -> Result<RunAdmission, ManagementError> {
        let mut result = self.submit(request, actor, definition_digest)?;
        if let Some(binding) = admission {
            if let Some(existing) = result.snapshot.admission.as_ref()
                && existing != binding
            {
                return Err(ManagementError::conflict(
                    "port_admission_mismatch",
                    "execution port returned a different target admission",
                ));
            }
            result.snapshot.admission = Some(binding.clone());
        }
        Ok(result)
    }

    fn apply_command(&self, context: CommandContext)
    -> Result<CommandApplication, ManagementError>;

    /// Applies a command while allowing an execution adapter to durably record
    /// an operation intent immediately before crossing a mutating boundary.
    ///
    /// Existing adapters inherit the direct command path. Live adapters use
    /// this hook to preserve one operation identity across transport errors.
    fn apply_command_with_intent(
        &self,
        context: CommandContext,
        _record_intent: &dyn Fn(PendingOperation) -> Result<(), ManagementError>,
    ) -> Result<CommandApplication, ManagementError> {
        self.apply_command(context)
    }
}

#[derive(Clone, Debug)]
pub struct RunAdmission {
    pub snapshot: RunSnapshot,
    pub initial_events: Vec<RunEvent>,
}

#[derive(Clone, Debug)]
pub struct CommandContext {
    pub request: CommandRequest,
    pub snapshot: RunSnapshot,
    pub actor: AuthContext,
}

#[derive(Clone, Debug)]
pub struct CommandApplication {
    pub snapshot: RunSnapshot,
    pub outcome: super::super::contract::CommandOutcome,
    pub reason_code: String,
}
