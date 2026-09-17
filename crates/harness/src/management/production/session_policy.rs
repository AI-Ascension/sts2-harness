// SPDX-License-Identifier: MIT

use super::*;

impl ProductionLiveWorkflowSession {
    /// Admission check for one inference request.
    ///
    /// A policy that was adopted after this session launched is picked up here,
    /// because no inference result exists yet that could have been produced under
    /// the previous policy; the adopted identity becomes the binding this
    /// request's result is validated against. A session that already discarded
    /// an in-flight result is fenced and stays fenced until a fresh live session
    /// is opened.
    pub(super) fn admit_active_policy_binding(&mut self) -> Result<(), ManagementError> {
        if self.policy_change_fenced {
            return Err(policy_changed_error());
        }
        if self.active_policy_binding.is_none() {
            return Err(missing_binding_error());
        }
        self.active_policy_binding = Some(self.load_active_policy_binding()?);
        Ok(())
    }

    /// Checks the active binding again before an inference result is consumed. A
    /// synchronous provider call cannot be cancelled after admission; if
    /// adoption changes in flight, the returned decision is discarded and later
    /// requests remain fenced until a fresh live session is opened.
    pub(super) fn assert_active_policy_binding_current(&mut self) -> Result<(), ManagementError> {
        let (expected_sha256, expected_generation) = self
            .active_policy_binding
            .as_ref()
            .ok_or_else(missing_binding_error)?;
        let (actual_sha256, actual_generation) = self.load_active_policy_binding()?;
        if actual_sha256 != *expected_sha256 || actual_generation != *expected_generation {
            self.policy_change_fenced = true;
            return Err(policy_changed_error());
        }
        Ok(())
    }

    fn load_active_policy_binding(&self) -> Result<(String, u64), ManagementError> {
        let current = self.provider_policy.load_active_policy(
            &self.actor,
            &self.request,
            &self.authority_binding.run_id,
            &self.definition,
            &self.provider_capabilities,
        )?;
        Ok((current.policy_sha256, current.adoption_generation))
    }
}

fn policy_changed_error() -> ManagementError {
    ManagementError::conflict(
        "provider_session_policy_changed",
        "active provider policy changed; inference results are discarded and a fresh live session is required",
    )
}

fn missing_binding_error() -> ManagementError {
    ManagementError::unavailable(
        "provider_session_policy_unavailable",
        "live session has no retained active-policy identity",
    )
}
