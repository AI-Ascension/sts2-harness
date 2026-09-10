// SPDX-License-Identifier: MIT

use super::super::super::action_envelope::validate_canonical_action_envelope;
use super::super::core::{
    ExecutionLineage, valid_catalog_raw, valid_digest, valid_id, valid_reference,
};
use super::super::error::ExecutionStoreError;
use serde_json::Value;
use sha2::Digest;

/// The recovery sideband caps the encoded canonical action at 65,536 bytes.  The decoded
/// canonical JSON is deliberately kept below that bound so an invalid row can be rejected before
/// it is copied into a transport request or a recovery frame.
pub const MAX_OPERATION_ACTION_BYTES: usize = 65_536;
pub const MAX_ORIGINAL_CONTEXT_BYTES: usize = 4_096;

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
    /// Canonical allocation identity at the original boundary. Historical recovery must never
    /// replace this with the context of a fresh lease or boot.
    pub original_context: Option<Vec<u8>>,
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
            original_context: None,
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
        let operation_id = operation_id.into();
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
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let intent = Self {
            lineage,
            operation_id,
            state_id: state_id.into(),
            generation,
            action_id,
            action_kind: Some(action_kind),
            action_payload: Some(action_payload),
            payload_digest,
            input_digest: input_digest.into(),
            catalog_digest,
            catalog_raw,
            original_context: None,
        };
        if intent.lineage.validate().is_err()
            || !valid_uuid_v4(&intent.operation_id)
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

    pub fn with_original_context(
        mut self,
        original_context: Vec<u8>,
    ) -> Result<Self, ExecutionStoreError> {
        if !valid_original_context(&original_context) {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        self.original_context = Some(original_context);
        Ok(self)
    }

    pub(crate) fn with_optional_original_context(
        self,
        original_context: Option<Vec<u8>>,
    ) -> Result<Self, ExecutionStoreError> {
        match original_context {
            Some(value) => self.with_original_context(value),
            None => Ok(self),
        }
    }

    pub fn has_durable_action(&self) -> bool {
        self.action_kind.is_some() && self.action_payload.is_some()
    }
}

fn valid_uuid_v4(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && value.as_bytes().get(14) == Some(&b'4')
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_original_context(value: &[u8]) -> bool {
    if value.is_empty() || value.len() > MAX_ORIGINAL_CONTEXT_BYTES {
        return false;
    }
    let Ok(decoded) = serde_json::from_slice::<Value>(value) else {
        return false;
    };
    decoded.is_object()
        && serde_json::to_vec(&decoded)
            .ok()
            .is_some_and(|canonical| canonical == value)
}
