// SPDX-License-Identifier: MIT

use super::*;
use crate::{BoundDecision, ExecutionStore, ProviderReservationState, StoredDecision};

impl LifecycleOwner {
    /// Repairs journal metadata from the existing completed result transaction. The broker remains
    /// held after restart; this is neither native reconciliation nor permission for a new send.
    /// The trusted consume port must explicitly authorize this historical manifest under current
    /// scheduler/game authority before the result can be returned.
    pub fn reconcile_stored(
        &mut self,
        manifest: &InvocationManifest,
        input: &[u8],
        store: &ExecutionStore,
    ) -> Result<BoundDecision, LifecycleError> {
        self.check()?;
        self.check_store(store)?;
        let index = self
            .snapshot
            .entries
            .iter()
            .position(|entry| entry.manifest == *manifest)
            .ok_or(LifecycleError::Held)?;
        let entry = &self.snapshot.entries[index];
        if !entry.possible_write
            || !matches!(
                entry.phase,
                LifecyclePhase::Sent | LifecyclePhase::Unknown | LifecyclePhase::Completed
            )
        {
            return Err(LifecycleError::Held);
        }
        let (result, bytes, digest) = checked_result(manifest, store)?;
        if entry
            .result_digest
            .as_ref()
            .is_some_and(|old| old != &digest)
            || entry
                .result_ref
                .as_ref()
                .is_some_and(|old| Some(old) != result.reference.result_ref.as_ref())
        {
            return Err(LifecycleError::Corrupt);
        }
        let decision = super::validation::response(manifest, input, &bytes)?;
        let authority = self.authority.clone();
        let _guard = authority
            .consume(manifest, &digest)
            .map_err(|_| LifecycleError::Fenced)?;
        self.bind_store(store);
        self.snapshot.entries[index].phase = LifecyclePhase::Completed;
        self.snapshot.entries[index].result_ref = result.reference.result_ref;
        self.snapshot.entries[index].result_digest = Some(digest);
        self.persist()?;
        Ok(decision)
    }
}

pub(super) fn checked_result(
    manifest: &InvocationManifest,
    store: &ExecutionStore,
) -> Result<(StoredDecision, Vec<u8>, String), LifecycleError> {
    let result = store
        .decision(&manifest.execution_id)
        .map_err(|_| LifecycleError::Held)?;
    let reservation = store
        .provider_reservation(&manifest.reservation_id)
        .map_err(|_| LifecycleError::Held)?;
    let mut expected = manifest.reservation()?;
    expected.state = ProviderReservationState::Completed;
    expected.actual_units = reservation.actual_units;
    if reservation != expected
        || reservation
            .actual_units
            .is_none_or(|units| units == 0 || units > manifest.reserved_units)
        || !result.completed
        || result.unknown
        || result.provider_reservation_id.as_deref() != Some(manifest.reservation_id.as_str())
    {
        return Err(LifecycleError::Held);
    }
    let bytes = result.result_payload.clone().ok_or(LifecycleError::Held)?;
    let digest = crate::sha256_hex(&bytes);
    let mut reference = manifest.decision()?;
    reference.result_ref = result.reference.result_ref.clone();
    reference.result_digest = Some(digest.clone());
    if result.reference != reference
        || reference
            .result_ref
            .as_ref()
            .is_none_or(|value| !types::id(value))
    {
        return Err(LifecycleError::Corrupt);
    }
    Ok((result, bytes, digest))
}
