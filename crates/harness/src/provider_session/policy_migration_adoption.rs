// SPDX-License-Identifier: MIT

use super::*;

impl SessionPolicyMigrationProposal {
    /// Adopt a caller-supplied target policy after approval.
    ///
    /// The target must already be bounded by the supplied owner descriptor; this API never derives
    /// a clamped target and refuses any target that still violates an executable limit.
    pub fn adopt(
        &mut self,
        target: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
        approval_ref: &str,
    ) -> Result<ProviderSessionPolicy, SessionPolicyMigrationError> {
        let target_bytes =
            serde_json::to_vec(target).map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        self.adopt_with_digest(
            target,
            capabilities,
            approval_ref,
            crate::sha256_hex(target_bytes),
        )
    }

    /// Adopt a target while binding the receipt to the exact bytes retained by its owner.
    ///
    /// The policy is decoded for semantic admission, but its original bytes remain unchanged and
    /// define its identity. This keeps pretty-printed and compact uploads distinct across restart.
    pub fn adopt_retained_bytes(
        &mut self,
        target_bytes: &[u8],
        capabilities: &NativeCapabilities,
        approval_ref: &str,
    ) -> Result<ProviderSessionPolicy, SessionPolicyMigrationError> {
        let target: ProviderSessionPolicy = serde_json::from_slice(target_bytes)
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        self.adopt_with_digest(
            &target,
            capabilities,
            approval_ref,
            crate::sha256_hex(target_bytes),
        )
    }

    fn adopt_with_digest(
        &mut self,
        target: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
        approval_ref: &str,
        target_sha256: String,
    ) -> Result<ProviderSessionPolicy, SessionPolicyMigrationError> {
        self.validate()?;
        if capabilities.binding.descriptor_sha256 != self.target_capabilities_sha256 {
            return Err(SessionPolicyMigrationError::InvalidCapabilities);
        }
        if self.state != SessionPolicyMigrationState::Approved
            || self.approval_ref.as_deref() != Some(approval_ref)
        {
            return Err(SessionPolicyMigrationError::PermissionDenied);
        }
        target
            .validate_schema()
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        if target.policy_id != self.source_policy_id
            || target.version <= self.source_policy_version
            || target.epoch <= self.adopted_source_epoch()
        {
            return Err(SessionPolicyMigrationError::InvalidProposal);
        }
        if let Some(violation) = target.capability_limit_violations(capabilities).first() {
            return Err(SessionPolicyMigrationError::CapabilityLimitExceeded {
                limit: violation.limit.clone(),
                requested: violation.requested,
                effective: violation.effective,
            });
        }
        self.adopted_policy_sha256 = Some(target_sha256);
        self.state = SessionPolicyMigrationState::Adopted;
        self.validate()?;
        Ok(target.clone())
    }
}
