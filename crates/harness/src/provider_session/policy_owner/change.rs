// SPDX-License-Identifier: MIT

use super::*;

impl ProviderSessionPolicyOwner {
    pub(super) fn change_if<T>(
        &self,
        f: impl FnOnce(&mut Journal) -> Result<(T, bool), ProviderSessionPolicyOwnerError>,
    ) -> Result<T, ProviderSessionPolicyOwnerError> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let mut candidate = journal.clone();
        let (result, changed) = f(&mut candidate)?;
        if changed {
            persist_candidate(&self.store, &mut journal, candidate)?;
        }
        Ok(result)
    }
}
