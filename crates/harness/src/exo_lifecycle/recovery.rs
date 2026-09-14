// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::{HistoryItem, HistoryItemKind, NativeOperationState};
use crate::{BoundDecision, ExecutionStore, ProviderFailureClass};

impl LifecycleOwner {
    /// Reads a completed result only through the current authority fence. No send is possible here.
    pub fn stored(
        &mut self,
        manifest: &InvocationManifest,
        input: &[u8],
        store: &ExecutionStore,
    ) -> Result<BoundDecision, LifecycleError> {
        self.check()?;
        let entry = self
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.manifest == *manifest)
            .ok_or(LifecycleError::Held)?;
        if entry.phase != LifecyclePhase::Completed {
            return Err(LifecycleError::Held);
        }
        self.validate_broker(manifest, NativeOperationState::Completed)?;
        let result = store
            .decision(&manifest.execution_id)
            .map_err(|_| LifecycleError::Held)?;
        let mut expected = manifest.decision()?;
        expected.result_ref = entry.result_ref.clone();
        expected.result_digest = entry.result_digest.clone();
        if !result.completed
            || result.unknown
            || result.reference != expected
            || result.provider_reservation_id.as_deref() != Some(&manifest.reservation_id)
        {
            return Err(LifecycleError::Held);
        }
        let bytes = result.result_payload.ok_or(LifecycleError::Held)?;
        let digest = crate::sha256_hex(&bytes);
        if entry.result_digest.as_deref() != Some(&digest) {
            return Err(LifecycleError::Corrupt);
        }
        let decision = super::validation::response(manifest, input, &bytes)?;
        let _guard = self.authority.consume(manifest, &digest)?;
        Ok(decision)
    }

    /// The caller may cancel/revoke through its authority owner while this handle is pending.
    pub fn poll<H: EffectHandle>(
        &mut self,
        inflight: &mut InFlight<H>,
        store: &mut ExecutionStore,
    ) -> Result<Option<BoundDecision>, LifecycleError> {
        self.check()?;
        if inflight.settled
            || inflight.owner_epoch != self.snapshot.claim_epoch
            || !std::sync::Arc::ptr_eq(&inflight.instance, &self.instance)
        {
            return Err(LifecycleError::Held);
        }
        let entry = self
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.manifest == inflight.manifest)
            .ok_or(LifecycleError::Held)?;
        if entry.phase != LifecyclePhase::Sent {
            return Err(LifecycleError::Held);
        }
        let completion = match inflight.handle.poll() {
            Ok(None) => return Ok(None),
            Ok(Some(completion)) => completion,
            Err(_) => {
                inflight.settled = true;
                self.hold(&inflight.manifest, store, LifecyclePhase::Unknown)?;
                return Err(LifecycleError::Unknown);
            }
        };
        inflight.settled = true;
        let result = self.finish(&inflight.manifest, &inflight.input, completion, store);
        if let Err(error) = result {
            if !self.poisoned {
                self.hold(
                    &inflight.manifest,
                    store,
                    if error == LifecycleError::Fenced {
                        LifecyclePhase::Fenced
                    } else {
                        LifecyclePhase::Unknown
                    },
                )?;
            }
            return Err(error);
        }
        result.map(Some)
    }

    pub(super) fn hold(
        &mut self,
        manifest: &InvocationManifest,
        store: &mut ExecutionStore,
        phase: LifecyclePhase,
    ) -> Result<(), LifecycleError> {
        let index = self
            .snapshot
            .entries
            .iter()
            .position(|entry| entry.manifest == *manifest)
            .ok_or(LifecycleError::Held)?;
        self.poisoned = true;
        store
            .mark_provider_unknown(&manifest.reservation_id, ProviderFailureClass::Outage, None)
            .map_err(|_| LifecycleError::Held)?;
        self.broker
            .mark_unknown(&self.token, &manifest.operation_id)
            .map_err(|_| LifecycleError::Held)?;
        self.snapshot.entries[index].phase = phase;
        self.persist()?;
        self.poisoned = false;
        Ok(())
    }

    fn finish(
        &mut self,
        manifest: &InvocationManifest,
        input: &[u8],
        completion: EffectCompletion,
        store: &mut ExecutionStore,
    ) -> Result<BoundDecision, LifecycleError> {
        self.validate_broker(manifest, NativeOperationState::Sent)?;
        completion
            .native
            .as_ref()
            .filter(|native| native.valid())
            .ok_or(LifecycleError::Unknown)?;
        let units = completion
            .actual_units
            .filter(|units| *units > 0 && *units <= manifest.reserved_units)
            .ok_or(LifecycleError::Unknown)?;
        if !types::id(&completion.result_ref) {
            return Err(LifecycleError::Invalid);
        }
        let decision = super::validation::response(manifest, input, &completion.response)?;
        let digest = crate::sha256_hex(&completion.response);
        let authority = self.authority.clone();
        let _guard = authority
            .consume(manifest, &digest)
            .map_err(|_| LifecycleError::Fenced)?;
        store
            .complete_provider_with_result(
                &manifest.reservation_id,
                &completion.result_ref,
                &digest,
                &completion.response,
                units,
            )
            .map_err(|_| LifecycleError::Held)?;
        // Once the result transaction succeeds, every remaining failure must require reopening.
        self.poisoned = true;
        self.complete_metadata(manifest, completion, digest)?;
        self.poisoned = false;
        Ok(decision)
    }

    fn complete_metadata(
        &mut self,
        manifest: &InvocationManifest,
        completion: EffectCompletion,
        digest: String,
    ) -> Result<(), LifecycleError> {
        let native = completion.native.as_ref().ok_or(LifecycleError::Unknown)?;
        let sequence = self
            .snapshot
            .broker
            .histories
            .get(&manifest.binding_id)
            .and_then(|items| items.last())
            .map_or(Some(1), |item| item.sequence.checked_add(1))
            .ok_or(LifecycleError::Capacity)?;
        self.broker
            .complete_turn(
                &self.token,
                &manifest.operation_id,
                &native.turn_id,
                HistoryItem {
                    item_ref: completion.result_ref.clone(),
                    turn_ref: native.turn_id.clone(),
                    sequence,
                    kind: HistoryItemKind::ValidatedDecision,
                    content_ref: Some(completion.result_ref.clone()),
                    redacted: true,
                },
            )
            .map_err(|_| LifecycleError::Held)?;
        let entry = self
            .snapshot
            .entries
            .iter_mut()
            .find(|entry| entry.manifest == *manifest)
            .ok_or(LifecycleError::Held)?;
        entry.phase = LifecyclePhase::Completed;
        entry.native = completion.native;
        entry.result_ref = Some(completion.result_ref);
        entry.result_digest = Some(digest);
        self.persist()?;
        Ok(())
    }
}
