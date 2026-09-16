// SPDX-License-Identifier: MIT

use super::*;

/// Fail-closed default for a management composition that has not attached the
/// durable saved-policy owner.
pub struct UnavailableLiveProviderPolicyPort;

impl LiveProviderPolicyPort for UnavailableLiveProviderPolicyPort {
    fn load_active_policy(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition: &crate::workflow::WorkflowDefinition,
        _capabilities: &crate::provider_session::NativeCapabilities,
    ) -> Result<ProviderSessionPolicyBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "provider_session_policy_owner_unavailable",
            "live provider admission requires an attached adopted provider-session policy owner",
        ))
    }
}
