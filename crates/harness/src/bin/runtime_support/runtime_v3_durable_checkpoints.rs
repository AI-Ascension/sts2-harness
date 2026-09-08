// SPDX-License-Identifier: MIT

#[cfg(test)]
use serde_json::Value;
use sts2_harness::{CatalogEvidence, Checkpoint, EpisodeObservation};

use super::{DurableHandle, sha256_bytes};

impl DurableHandle {
    /// Requires the first fresh host observation after an explicit resume to equal the last
    /// durable public boundary. A mismatch cannot be turned into a new provider decision.
    pub(in super::super) fn verify_resume_boundary_with_catalog(
        &self,
        observation: &EpisodeObservation,
        catalog_raw: &[u8],
    ) -> Result<(), String> {
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 resume observation: {error}"))?;
        let mut boundary = self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))?;
        let Some(expected) = boundary.as_ref() else {
            return Ok(());
        };
        if catalog_raw.is_empty() || catalog_raw.len() > sts2_harness::MAX_CATALOG_BYTES {
            return Err(String::from(
                "runtime-v3 fresh observation has an invalid legal-action catalog",
            ));
        }
        if expected.lineage != self.lineage
            || expected.fingerprint != self.fingerprint
            || expected.state_id != observation.state_id()
            || expected.generation != observation.generation()
            || expected.observation != observation_bytes
            || expected.catalog_raw.as_deref() != Some(catalog_raw)
            || expected.legal_actions_digest != sha256_bytes(catalog_raw)
        {
            return Err(String::from(
                "runtime-v3 fresh observation does not match the verified resume boundary",
            ));
        }
        *boundary = None;
        Ok(())
    }

    #[cfg(test)]
    pub(in super::super) fn verify_resume_boundary(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 resume observation: {error}"))?;
        let mut boundary = self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))?;
        let Some(expected) = boundary.as_ref() else {
            return Ok(());
        };
        if expected.lineage != self.lineage
            || expected.fingerprint != self.fingerprint
            || expected.state_id != observation.state_id()
            || expected.generation != observation.generation()
            || expected.observation != observation_bytes
        {
            return Err(String::from(
                "runtime-v3 fresh observation does not match the verified resume boundary",
            ));
        }
        *boundary = None;
        Ok(())
    }

    /// Makes the latest checkpoint the next resume boundary after recovery has produced an
    /// authoritative observation. This is deliberately called only after all pending mutations
    /// have been reconciled.
    pub(in super::super) fn refresh_resume_boundary(&self) -> Result<(), String> {
        let checkpoint = {
            let store = super::super::worker_store::try_lock_recovery(&self.store)?;
            super::super::worker_store::snapshot(&store, &self.lineage, &self.fingerprint)?
                .episode
                .last_checkpoint
        };
        *self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))? =
            checkpoint;
        Ok(())
    }

    #[cfg(test)]
    pub(in super::super) fn checkpoint(
        &self,
        observation: &EpisodeObservation,
        payloads: &Value,
    ) -> Result<(), String> {
        let catalog_raw = serde_json::to_vec(payloads)
            .map_err(|error| format!("cannot encode runtime-v3 checkpoint catalog: {error}"))?;
        self.checkpoint_raw(observation, &catalog_raw)
    }

    pub(in super::super) fn checkpoint_raw(
        &self,
        observation: &EpisodeObservation,
        catalog_raw: &[u8],
    ) -> Result<(), String> {
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 checkpoint: {error}"))?;
        let legal_actions_digest = sha256_bytes(catalog_raw);
        let mut sequence = self
            .next_checkpoint
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 checkpoint sequence is already borrowed"))?;
        let mut store = super::try_lock(&self.store)?;
        let stored =
            super::super::worker_store::snapshot(&store, &self.lineage, &self.fingerprint)?;
        let stored_next =
            stored
                .episode
                .last_checkpoint
                .as_ref()
                .map_or(Ok(0_u64), |checkpoint| {
                    checkpoint
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))
                })?;
        if *sequence != stored_next {
            return Err(String::from(
                "runtime-v3 checkpoint sequence changed in the shared store",
            ));
        }
        let checkpoint = Checkpoint::new_with_catalog(
            self.lineage.clone(),
            *sequence,
            observation.state_id(),
            observation.generation(),
            self.fingerprint.clone(),
            observation_bytes,
            CatalogEvidence::new(legal_actions_digest, Some(catalog_raw.to_vec())),
        )
        .map_err(|error| format!("runtime-v3 checkpoint is invalid: {error}"))?;
        store
            .save_checkpoint(&checkpoint)
            .map_err(|error| format!("cannot persist runtime-v3 checkpoint: {error}"))?;
        *sequence = sequence
            .checked_add(1)
            .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))?;
        Ok(())
    }
}
