// SPDX-License-Identifier: MIT

use sts2_harness::{CatalogEvidence, Checkpoint, EpisodeObservation};

use super::{DurableHandle, sha256_bytes};

impl DurableHandle {
    pub(in super::super) fn clear_resume_boundary(&self) -> Result<(), String> {
        *self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))? = None;
        Ok(())
    }

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

    /// Makes the latest checkpoint the next resume boundary after recovery has produced an
    /// authoritative observation. This is deliberately called only after all pending mutations
    /// have been reconciled.
    pub(in super::super) fn refresh_resume_boundary(&self) -> Result<(), String> {
        let checkpoint = self
            .store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .last_checkpoint(&self.lineage.episode_id)
            .map_err(|error| format!("cannot refresh runtime-v3 resume boundary: {error}"))?;
        *self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))? =
            checkpoint;
        Ok(())
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
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .save_checkpoint(&checkpoint)
            .map_err(|error| format!("cannot persist runtime-v3 checkpoint: {error}"))?;
        *sequence = sequence
            .checked_add(1)
            .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))?;
        Ok(())
    }
}
