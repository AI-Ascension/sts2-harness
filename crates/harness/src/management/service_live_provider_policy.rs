// SPDX-License-Identifier: MIT

use crate::provider_session::{NativeCapabilities, ProviderSessionPolicy};
use crate::workflow::WorkflowDefinition;

use super::*;

/// Immutable result from the trusted saved-policy owner. This crosses into
/// live provider composition only after the owner has authenticated the actor,
/// scope and explicit adopted revision; source policy bytes and proposal
/// history never enter a workflow snapshot or provider transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSessionPolicyBinding {
    pub policy: ProviderSessionPolicy,
    pub policy_sha256: String,
    pub active_revision: u64,
}

/// Trusted live-policy boundary. A live factory calls this after it has its
/// gateway/current-context fence and before it retains context or opens a
/// provider session, including restart recovery.
pub trait LiveProviderPolicyPort: Send + Sync {
    fn load_active_policy(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        workflow_run_id: &str,
        definition: &WorkflowDefinition,
        capabilities: &NativeCapabilities,
    ) -> Result<ProviderSessionPolicyBinding, ManagementError>;
}

/// Fail-closed default for a management composition that has not attached the
/// durable saved-policy owner.
pub struct UnavailableLiveProviderPolicyPort;

impl LiveProviderPolicyPort for UnavailableLiveProviderPolicyPort {
    fn load_active_policy(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _workflow_run_id: &str,
        _definition: &crate::workflow::WorkflowDefinition,
        _capabilities: &crate::provider_session::NativeCapabilities,
    ) -> Result<ProviderSessionPolicyBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "provider_session_policy_owner_unavailable",
            "live provider admission requires an attached adopted provider-session policy owner",
        ))
    }
}
