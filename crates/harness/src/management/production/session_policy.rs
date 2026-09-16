// SPDX-License-Identifier: MIT

use super::*;

impl ProductionLiveWorkflowSession {
    /// Checks the active binding before inference and again before its result is
    /// consumed. A synchronous provider call cannot be cancelled after admission;
    /// if adoption changes in flight, the returned decision is discarded and
    /// later requests remain fenced until a fresh live session is opened.
    pub(super) fn assert_active_policy_binding_current(&self) -> Result<(), ManagementError> {
        let (expected_sha256, expected_generation) =
            self.active_policy_binding.as_ref().ok_or_else(|| {
                ManagementError::unavailable(
                    "provider_session_policy_unavailable",
                    "live session has no retained active-policy identity",
                )
            })?;
        let current = self.provider_policy.load_active_policy(
            &self.actor,
            &self.request,
            &self.authority_binding.run_id,
            &self.definition,
            &self.provider_capabilities,
        )?;
        if &current.policy_sha256 != expected_sha256
            || current.adoption_generation != *expected_generation
        {
            return Err(ManagementError::conflict(
                "provider_session_policy_changed",
                "active provider policy changed; inference results are discarded and a fresh live session is required",
            ));
        }
        Ok(())
    }
}
