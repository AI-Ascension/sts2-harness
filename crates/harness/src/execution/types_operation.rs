// SPDX-License-Identifier: MIT

use super::super::super::action_envelope::validate_canonical_action_envelope;
use super::super::core::{
    ExecutionLineage, valid_catalog_raw, valid_digest, valid_id, valid_original_context_raw,
    valid_reference,
};
use super::super::error::ExecutionStoreError;
use sha2::Digest;

/// The recovery sideband caps the encoded canonical action at 65,536 bytes.  The decoded
/// canonical JSON is deliberately kept below that bound so an invalid row can be rejected before
/// it is copied into a transport request or a recovery frame.
pub const MAX_OPERATION_ACTION_BYTES: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationIntent {
    pub lineage: ExecutionLineage,
    pub operation_id: String,
    pub state_id: String,
    pub generation: u64,
    pub action_id: String,
    /// The exact semantic kind recorded by the runtime-v3 catalog. `None` is retained only for
    /// rows created before the action identity migration; such rows are never mutation-ready.
    pub action_kind: Option<String>,
    /// Canonical `{"action":...,"action_id":...}` bytes for the operation. `None` is retained
    /// only for legacy rows so history can be inspected without guessing a replacement action.
    pub action_payload: Option<Vec<u8>>,
    pub payload_digest: String,
    pub input_digest: String,
    /// Digest of the complete legal-action catalog at the original boundary. It is required by
    /// the recovery sideband and is absent only on pre-migration rows.
    pub catalog_digest: Option<String>,
    /// Exact legal_actions array bytes at the original boundary. Legacy rows may omit this
    /// retention field, but a present value is always checked against `catalog_digest`.
    pub catalog_raw: Option<Vec<u8>>,
    /// Exact closed original host/lease context captured before dispatch. Legacy rows retain
    /// `None` and are intentionally blocked from historical mutation reconciliation.
    pub original_context_raw: Option<Vec<u8>>,
}

impl OperationIntent {
    pub fn new(
        lineage: ExecutionLineage,
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
        payload_digest: impl Into<String>,
        input_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let intent = Self {
            lineage,
            operation_id: operation_id.into(),
            state_id: state_id.into(),
            generation,
            action_id: action_id.into(),
            action_kind: None,
            action_payload: None,
            payload_digest: payload_digest.into(),
            input_digest: input_digest.into(),
            catalog_digest: None,
            catalog_raw: None,
            original_context_raw: None,
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Creates an operation intent with the complete typed action identity. The digest is checked
    /// against the bytes before the value can reach SQLite, preventing a large or mismatched
    /// payload from being copied into the durable ledger.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_action(
        lineage: ExecutionLineage,
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
        action_kind: impl Into<String>,
        action_payload: Vec<u8>,
        payload_digest: impl Into<String>,
        input_digest: impl Into<String>,
        catalog_digest: Option<String>,
    ) -> Result<Self, ExecutionStoreError> {
        Self::new_with_action_and_catalog(
            lineage,
            operation_id,
            state_id,
            generation,
            action_id,
            action_kind,
            action_payload,
            payload_digest,
            input_digest,
            catalog_digest,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_action_and_catalog(
        lineage: ExecutionLineage,
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
        action_kind: impl Into<String>,
        action_payload: Vec<u8>,
        payload_digest: impl Into<String>,
        input_digest: impl Into<String>,
        catalog_digest: Option<String>,
        catalog_raw: Option<Vec<u8>>,
    ) -> Result<Self, ExecutionStoreError> {
        Self::new_with_action_and_catalog_and_context(
            lineage,
            operation_id,
            state_id,
            generation,
            action_id,
            action_kind,
            action_payload,
            payload_digest,
            input_digest,
            catalog_digest,
            catalog_raw,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_action_and_catalog_and_context(
        lineage: ExecutionLineage,
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
        action_kind: impl Into<String>,
        action_payload: Vec<u8>,
        payload_digest: impl Into<String>,
        input_digest: impl Into<String>,
        catalog_digest: Option<String>,
        catalog_raw: Option<Vec<u8>>,
        original_context_raw: Option<Vec<u8>>,
    ) -> Result<Self, ExecutionStoreError> {
        let action_id = action_id.into();
        let action_kind = action_kind.into();
        let payload_digest = payload_digest.into();
        if action_payload.is_empty() || action_payload.len() > MAX_OPERATION_ACTION_BYTES {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let calculated = format!("{:x}", sha2::Sha256::digest(&action_payload));
        if calculated != payload_digest
            || !validate_canonical_action_envelope(
                &action_id,
                &action_payload,
                MAX_OPERATION_ACTION_BYTES,
            )
            || !valid_reference(&action_kind)
            || catalog_digest
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
            || catalog_raw.as_ref().is_some_and(|raw| {
                catalog_digest
                    .as_deref()
                    .is_none_or(|digest| !valid_catalog_raw(raw, digest))
            })
            || (catalog_raw.is_some() && catalog_digest.is_none())
            || original_context_raw
                .as_ref()
                .is_some_and(|raw| !valid_original_context_raw(raw))
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let intent = Self {
            lineage,
            operation_id: operation_id.into(),
            state_id: state_id.into(),
            generation,
            action_id,
            action_kind: Some(action_kind),
            action_payload: Some(action_payload),
            payload_digest,
            input_digest: input_digest.into(),
            catalog_digest,
            catalog_raw,
            original_context_raw,
        };
        intent.validate()?;
        Ok(intent)
    }

    pub(crate) fn validate(&self) -> Result<(), ExecutionStoreError> {
        if self.lineage.validate().is_err()
            || !valid_id(&self.operation_id)
            || !valid_id(&self.state_id)
            || !valid_id(&self.action_id)
            || self.generation > 9_007_199_254_740_991
            || !valid_reference(&self.payload_digest)
            || !valid_reference(&self.input_digest)
            || self
                .original_context_raw
                .as_ref()
                .is_some_and(|raw| !valid_original_context_raw(raw))
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        match (&self.action_kind, &self.action_payload) {
            (None, None) => {
                if self.catalog_raw.is_some()
                    || self
                        .catalog_digest
                        .as_deref()
                        .is_some_and(|digest| !valid_digest(digest))
                {
                    return Err(ExecutionStoreError::InvalidOperation);
                }
            }
            (Some(action_kind), Some(action_payload)) => {
                if action_payload.is_empty()
                    || action_payload.len() > MAX_OPERATION_ACTION_BYTES
                    || !valid_reference(action_kind)
                    || format!("{:x}", sha2::Sha256::digest(action_payload)) != self.payload_digest
                    || !validate_canonical_action_envelope(
                        &self.action_id,
                        action_payload,
                        MAX_OPERATION_ACTION_BYTES,
                    )
                    || self
                        .catalog_digest
                        .as_deref()
                        .is_some_and(|digest| !valid_digest(digest))
                    || self.catalog_raw.as_ref().is_some_and(|raw| {
                        self.catalog_digest
                            .as_deref()
                            .is_none_or(|digest| !valid_catalog_raw(raw, digest))
                    })
                    || (self.catalog_raw.is_some() && self.catalog_digest.is_none())
                {
                    return Err(ExecutionStoreError::InvalidOperation);
                }
            }
            _ => return Err(ExecutionStoreError::InvalidOperation),
        }
        Ok(())
    }

    pub fn has_durable_action(&self) -> bool {
        self.action_kind.is_some() && self.action_payload.is_some()
    }
}
