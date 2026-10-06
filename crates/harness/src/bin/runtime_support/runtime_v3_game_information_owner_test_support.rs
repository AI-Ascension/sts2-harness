// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeGameInformationOwner {
    pub(super) fn test_adoption_read_guard(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, ()>, PolicyOwnerError> {
        self.test_adoption_barrier
            .read()
            .map_err(|_| PolicyOwnerError::Unavailable)
    }

    pub(super) fn test_adoption_write_guard(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, ()>, ManagementError> {
        self.test_adoption_barrier.write().map_err(|_| {
            ManagementError::store(
                "owner_test_barrier_unavailable",
                "owner test synchronization is unavailable",
            )
        })
    }

    pub(super) fn test_clock_after_adoption(&self) {
        if let Some(clock) = &self.test_policy_clock {
            clock.after_adoption();
        }
    }

    pub(super) fn test_clock_record_preflight_refusal(&self, error: &PolicyOwnerError) {
        if let Some(clock) = &self.test_policy_clock {
            clock.record_preflight_refusal(error);
        }
    }
}
