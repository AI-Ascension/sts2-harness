// SPDX-License-Identifier: MIT

use super::*;

pub struct UnavailableProviderSessionInspectionPort;

impl ProviderSessionInspectionPort for UnavailableProviderSessionInspectionPort {
    fn list(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionInspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "provider_session_inspection_unavailable",
            "provider-session inspection is not injected into the management adapter",
        ))
    }
}
