// SPDX-License-Identifier: MIT

use super::core::{
    ExecutionFingerprint, ExecutionLineage, MAX_PAYLOAD_BYTES, valid_catalog_raw, valid_id,
    valid_reference,
};
use super::error::ExecutionStoreError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    pub lineage: ExecutionLineage,
    pub sequence: u64,
    pub state_id: String,
    pub generation: u64,
    pub fingerprint: ExecutionFingerprint,
    pub observation: Vec<u8>,
    pub legal_actions_digest: String,
    /// Exact legal_actions array bytes from the authoritative response. Legacy checkpoints may
    /// omit this field, but new runtime-v3 checkpoints retain it for byte-identity recovery.
    pub catalog_raw: Option<Vec<u8>>,
}

impl Checkpoint {
    pub fn new(
        lineage: ExecutionLineage,
        sequence: u64,
        state_id: impl Into<String>,
        generation: u64,
        fingerprint: ExecutionFingerprint,
        observation: Vec<u8>,
        legal_actions_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        Self::new_with_optional_catalog(
            lineage,
            sequence,
            state_id,
            generation,
            fingerprint,
            observation,
            legal_actions_digest,
            None,
        )
    }

    pub fn new_with_catalog(
        lineage: ExecutionLineage,
        sequence: u64,
        state_id: impl Into<String>,
        generation: u64,
        fingerprint: ExecutionFingerprint,
        observation: Vec<u8>,
        legal_actions_digest: impl Into<String>,
        catalog_raw: Vec<u8>,
    ) -> Result<Self, ExecutionStoreError> {
        Self::new_with_optional_catalog(
            lineage,
            sequence,
            state_id,
            generation,
            fingerprint,
            observation,
            legal_actions_digest,
            Some(catalog_raw),
        )
    }

    pub(crate) fn new_with_optional_catalog(
        lineage: ExecutionLineage,
        sequence: u64,
        state_id: impl Into<String>,
        generation: u64,
        fingerprint: ExecutionFingerprint,
        observation: Vec<u8>,
        legal_actions_digest: impl Into<String>,
        catalog_raw: Option<Vec<u8>>,
    ) -> Result<Self, ExecutionStoreError> {
        let checkpoint = Self {
            lineage,
            sequence,
            state_id: state_id.into(),
            generation,
            fingerprint,
            observation,
            legal_actions_digest: legal_actions_digest.into(),
            catalog_raw,
        };
        checkpoint.validate(MAX_PAYLOAD_BYTES)?;
        Ok(checkpoint)
    }

    pub fn validate(&self, maximum: usize) -> Result<(), ExecutionStoreError> {
        self.lineage
            .validate()
            .map_err(|_| ExecutionStoreError::InvalidCheckpoint)?;
        self.fingerprint
            .validate()
            .map_err(|_| ExecutionStoreError::InvalidCheckpoint)?;
        if maximum == 0
            || maximum > MAX_PAYLOAD_BYTES
            || !valid_id(&self.state_id)
            || self.generation > 9_007_199_254_740_991
            || self.observation.is_empty()
            || self.observation.len() > maximum
            || !valid_reference(&self.legal_actions_digest)
            || self
                .catalog_raw
                .as_ref()
                .is_some_and(|raw| !valid_catalog_raw(raw, &self.legal_actions_digest))
        {
            return Err(ExecutionStoreError::InvalidCheckpoint);
        }
        Ok(())
    }
}
