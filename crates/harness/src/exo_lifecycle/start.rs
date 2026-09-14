// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::NativeOperationState;
use crate::{ExecutionFingerprint, ExecutionStore, ProviderReservationState};

impl LifecycleOwner {
    pub fn start<P: EffectPort>(
        &mut self,
        manifest: InvocationManifest,
        input: &[u8],
        store: &mut ExecutionStore,
        fingerprint: &ExecutionFingerprint,
        effect: &mut P,
    ) -> Result<StartOutcome<P::Handle>, LifecycleError> {
        self.check()?;
        super::validation::input(&manifest, input)?;
        if let Some(entry) = self
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.manifest.execution_id == manifest.execution_id)
        {
            if entry.manifest != manifest {
                return Err(LifecycleError::Stale);
            }
            return self
                .stored(&manifest, input, store)
                .map(StartOutcome::Stored);
        }
        self.admit_new(&manifest, store, fingerprint)?;
        let authority = self.authority.clone();
        let guard = authority.admit(&manifest)?;
        let permit = self.prepare_send(&manifest, store)?;
        let result = effect.try_start(permit, input);
        drop(guard);
        match result {
            Ok(handle) => Ok(StartOutcome::Started(Box::new(InFlight {
                handle,
                manifest,
                owner_epoch: self.snapshot.claim_epoch,
                input: input.to_vec(),
                settled: false,
                instance: self.instance.clone(),
            }))),
            Err(_) => {
                self.hold(&manifest, store, LifecyclePhase::Unknown)?;
                Err(LifecycleError::Unknown)
            }
        }
    }

    fn prepare_send(
        &mut self,
        manifest: &InvocationManifest,
        store: &mut ExecutionStore,
    ) -> Result<SendPermit, LifecycleError> {
        self.snapshot.entries.push(LifecycleEntry::prepared(
            manifest.clone(),
            self.snapshot.claim_epoch,
        ));
        self.persist()?;
        let reference = manifest.decision()?;
        let decision = store
            .record_decision(&reference)
            .map_err(|_| LifecycleError::Held)?;
        if decision.completed
            || decision.unknown
            || decision.reference != reference
            || decision.provider_reservation_id.is_some()
            || decision.result_payload.is_some()
        {
            return Err(LifecycleError::Held);
        }
        let expected = manifest.reservation()?;
        let reservation = store
            .reserve_provider(&expected)
            .map_err(|_| LifecycleError::Held)?;
        if reservation.state != ProviderReservationState::Reserved || reservation != expected {
            return Err(LifecycleError::Held);
        }
        let index = self.snapshot.entries.len() - 1;
        self.snapshot.entries[index].phase = LifecyclePhase::Admitted;
        self.persist()?;
        self.broker
            .mark_sent(&self.token, &manifest.operation_id)
            .map_err(|_| LifecycleError::Held)?;
        let revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(LifecycleError::Capacity)?;
        self.snapshot.entries[index].phase = LifecyclePhase::Sent;
        self.snapshot.entries[index].possible_write = true;
        self.snapshot.entries[index].permit_revision = Some(revision);
        self.persist()?;
        Ok(SendPermit {
            operation_id: manifest.operation_id.clone(),
            claim_epoch: self.snapshot.claim_epoch,
            revision,
        })
    }

    fn admit_new(
        &self,
        manifest: &InvocationManifest,
        store: &ExecutionStore,
        fingerprint: &ExecutionFingerprint,
    ) -> Result<(), LifecycleError> {
        if self.snapshot.entries.len()
            >= self
                .broker
                .policy()
                .max_completed_turns
                .min(MAX_LIFECYCLE_ENTRIES)
        {
            return Err(LifecycleError::Capacity);
        }
        if self.snapshot.entries.iter().any(|entry| {
            !matches!(
                entry.phase,
                LifecyclePhase::Completed | LifecyclePhase::FailedBeforeSend
            ) || entry.manifest.operation_id == manifest.operation_id
                || entry.manifest.reservation_id == manifest.reservation_id
        }) {
            return Err(LifecycleError::Held);
        }
        self.validate_broker(manifest, NativeOperationState::IntentPersisted)?;
        let episode = store
            .load_episode(&manifest.scope.episode_id)
            .map_err(|_| LifecycleError::Held)?;
        if episode.lineage != manifest.lineage()? {
            return Err(LifecycleError::Stale);
        }
        store
            .resume_for_decision(&manifest.scope.episode_id, fingerprint)
            .map_err(|_| LifecycleError::Held)?;
        // A result present without this journal's identity must not become a fresh send.
        match store.decision(&manifest.execution_id) {
            Err(crate::ExecutionStoreError::Missing) => Ok(()),
            _ => Err(LifecycleError::Held),
        }
    }
}
