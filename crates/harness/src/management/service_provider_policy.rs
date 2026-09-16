// SPDX-License-Identifier: MIT

use super::support::authorize;
use super::*;

impl ManagementService {
    pub fn with_provider_session_inspection_port(
        mut self,
        port: Arc<dyn ProviderSessionInspectionPort>,
    ) -> Self {
        self.provider_session_inspection = port;
        self
    }

    pub fn with_provider_session_policy_command_port(
        mut self,
        port: Arc<dyn super::super::provider_policy::ProviderSessionPolicyCommandPort>,
    ) -> Self {
        self.provider_session_policy = port;
        self
    }

    pub fn provider_session_policy_command_port(
        &self,
    ) -> &dyn super::super::provider_policy::ProviderSessionPolicyCommandPort {
        self.provider_session_policy.as_ref()
    }

    pub fn with_live_provider_policy_port(mut self, port: Arc<dyn LiveProviderPolicyPort>) -> Self {
        self.live_provider_policy = port;
        self
    }

    pub fn live_provider_policy_port(&self) -> &dyn LiveProviderPolicyPort {
        self.live_provider_policy.as_ref()
    }

    /// Returns the durable provider-session owner's current adopted binding
    /// and bounded history metadata. Policy bytes and credential references
    /// never leave the owner port.
    pub fn provider_session_policy(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ProviderSessionPolicyViewResponse, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let value = self.provider_session_policy.current(actor, &snapshot)?;
        if value.run_id != snapshot.workflow_run_id {
            return Err(ManagementError::conflict(
                "provider_session_policy_run_mismatch",
                "provider-session policy owner returned a different workflow run identity",
            ));
        }
        if value.history.len() > 64 || value.proposals.len() > 64 {
            return Err(ManagementError::invalid(
                "provider_session_policy_projection_capacity",
                "provider-session policy owner exceeded the bounded management projection",
            ));
        }
        Ok(ProviderSessionPolicyViewResponse {
            schema_version: PROVIDER_SESSION_POLICY_VIEW_SCHEMA_VERSION.to_owned(),
            operation: "current".to_owned(),
            value,
            effect_class: "local_metadata_only".to_owned(),
            inference_calls: 0,
            game_effects: 0,
        })
    }

    /// Applies one explicit saved-policy owner command through the
    /// authenticated, run-scoped adapter. The response contains only redacted
    /// metadata and the owner's resulting revision.
    pub fn provider_session_policy_command(
        &self,
        actor: &AuthContext,
        run_id: &str,
        command: super::super::provider_policy::ProviderSessionPolicyOwnerCommand,
    ) -> Result<ProviderSessionPolicyCommandResponse, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let operation = match &command {
            super::super::provider_policy::ProviderSessionPolicyOwnerCommand::Import { .. } => {
                "import"
            }
            super::super::provider_policy::ProviderSessionPolicyOwnerCommand::Propose {
                ..
            } => "propose",
            super::super::provider_policy::ProviderSessionPolicyOwnerCommand::Approve {
                ..
            } => "approve",
            super::super::provider_policy::ProviderSessionPolicyOwnerCommand::Adopt { .. }
            | super::super::provider_policy::ProviderSessionPolicyOwnerCommand::AdoptImported {
                ..
            } => "adopt",
        };
        let result = self
            .provider_session_policy
            .execute(actor, &snapshot, command)?;
        let value = self.provider_session_policy.current(actor, &snapshot)?;
        if value.run_id != snapshot.workflow_run_id
            || value.history.len() > 64
            || value.proposals.len() > 64
        {
            return Err(ManagementError::conflict(
                "provider_session_policy_projection_invalid",
                "provider-session policy owner returned an invalid bounded projection",
            ));
        }
        let (policy_sha256, proposal_sha256) = match result {
            super::super::provider_policy::ProviderSessionPolicyOwnerCommandResult::Imported {
                policy_sha256,
            } => (Some(policy_sha256), None),
            super::super::provider_policy::ProviderSessionPolicyOwnerCommandResult::Proposed {
                proposal_sha256,
            } => (None, Some(proposal_sha256)),
            super::super::provider_policy::ProviderSessionPolicyOwnerCommandResult::Approved => {
                (None, None)
            }
            super::super::provider_policy::ProviderSessionPolicyOwnerCommandResult::Adopted => (
                value.active.as_ref().map(|active| active.sha256.clone()),
                None,
            ),
        };
        Ok(ProviderSessionPolicyCommandResponse {
            schema_version: super::super::contract::PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION
                .to_owned(),
            operation: operation.to_owned(),
            revision: value.revision,
            policy_sha256,
            proposal_sha256,
            effect_class: "local_metadata_only".to_owned(),
            inference_calls: 0,
            game_effects: 0,
        })
    }
}
