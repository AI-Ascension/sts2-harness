// SPDX-License-Identifier: MIT

use super::super::core::{ExecutionLineage, valid_id, valid_reference};
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
        };
        if intent.lineage.validate().is_err()
            || !valid_id(&intent.operation_id)
            || !valid_id(&intent.state_id)
            || !valid_id(&intent.action_id)
            || intent.generation > 9_007_199_254_740_991
            || !valid_reference(&intent.payload_digest)
            || !valid_reference(&intent.input_digest)
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
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
        let action_kind = action_kind.into();
        let payload_digest = payload_digest.into();
        if action_payload.is_empty() || action_payload.len() > MAX_OPERATION_ACTION_BYTES {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let calculated = format!("{:x}", sha2::Sha256::digest(&action_payload));
        if calculated != payload_digest
            || !valid_reference(&action_kind)
            || catalog_digest
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let intent = Self {
            lineage,
            operation_id: operation_id.into(),
            state_id: state_id.into(),
            generation,
            action_id: action_id.into(),
            action_kind: Some(action_kind),
            action_payload: Some(action_payload),
            payload_digest,
            input_digest: input_digest.into(),
            catalog_digest,
        };
        if intent.lineage.validate().is_err()
            || !valid_id(&intent.operation_id)
            || !valid_id(&intent.state_id)
            || !valid_id(&intent.action_id)
            || intent.generation > 9_007_199_254_740_991
            || !valid_reference(&intent.payload_digest)
            || !valid_reference(&intent.input_digest)
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        Ok(intent)
    }

    pub fn has_durable_action(&self) -> bool {
        self.action_kind.is_some() && self.action_payload.is_some()
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
