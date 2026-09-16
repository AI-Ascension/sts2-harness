// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::Ordering;

impl ProviderSessionPolicyOwner {
    pub fn active(
        &self,
    ) -> Result<(ProviderSessionPolicy, String, u64), ProviderSessionPolicyOwnerError> {
        self.active_with_generation()
            .map(|(policy, digest, revision, _)| (policy, digest, revision))
    }

    /// Returns the active policy with an in-process generation that changes
    /// only when the persisted active binding changes. Journal-only edits do
    /// not invalidate live sessions.
    pub fn active_with_generation(
        &self,
    ) -> Result<(ProviderSessionPolicy, String, u64, u64), ProviderSessionPolicyOwnerError> {
        self.verify_lease()?;
        let journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let active = journal
            .active_sha256
            .as_ref()
            .ok_or(ProviderSessionPolicyOwnerError::NotAdopted)?;
        let record = journal
            .policies
            .iter()
            .find(|item| &item.sha256 == active)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let policy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let generation = self.adoption_generation.load(Ordering::Acquire);
        Ok((policy, active.clone(), journal.revision, generation))
    }
}
