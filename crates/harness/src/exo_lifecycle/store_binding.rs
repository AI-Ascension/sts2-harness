// SPDX-License-Identifier: MIT

use super::*;
use crate::{ExecutionStore, StoredDecision};
use std::sync::Arc;

impl LifecycleOwner {
    pub(super) fn check_store(&self, store: &ExecutionStore) -> Result<(), LifecycleError> {
        if self
            .store_instance
            .as_ref()
            .is_some_and(|expected| !Arc::ptr_eq(expected, store.incarnation()))
        {
            return Err(LifecycleError::Held);
        }
        Ok(())
    }

    pub(super) fn bind_store(&mut self, store: &ExecutionStore) {
        self.store_instance = Some(store.incarnation().clone());
    }

    pub(super) fn pending_store(
        &self,
        manifest: &InvocationManifest,
        store: &ExecutionStore,
    ) -> Result<(), LifecycleError> {
        self.check_store(store)?;
        let reservation = store
            .provider_reservation(&manifest.reservation_id)
            .map_err(|_| LifecycleError::Held)?;
        let decision = store
            .decision(&manifest.execution_id)
            .map_err(|_| LifecycleError::Held)?;
        if reservation != manifest.reservation()?
            || decision.reference != manifest.decision()?
            || decision.completed
            || decision.unknown
            || decision.result_payload.is_some()
            || decision.provider_reservation_id.as_deref() != Some(&manifest.reservation_id)
        {
            return Err(LifecycleError::Held);
        }
        Ok(())
    }
}

pub(super) fn completed_return(
    manifest: &InvocationManifest,
    completion: &EffectCompletion,
    digest: &str,
    result: &StoredDecision,
) -> Result<(), LifecycleError> {
    let mut reference = manifest.decision()?;
    reference.result_ref = Some(completion.result_ref.clone());
    reference.result_digest = Some(digest.to_owned());
    if result.reference != reference
        || !result.completed
        || result.unknown
        || result.provider_reservation_id.as_deref() != Some(&manifest.reservation_id)
        || result.result_payload.as_deref() != Some(completion.response.as_slice())
    {
        return Err(LifecycleError::Corrupt);
    }
    Ok(())
}
