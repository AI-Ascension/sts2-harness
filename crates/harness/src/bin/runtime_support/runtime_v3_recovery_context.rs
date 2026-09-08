// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::Digest;

use super::super::super::config::RuntimeConfig;
use super::super::allocation_context::RecoveryAuthority;

#[derive(Clone, Debug)]
pub(in super::super) struct RecoveryContext {
    pub(super) instance_id: String,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) mcp_session_id: String,
    current_fence: Value,
}

impl RecoveryContext {
    pub(in super::super) fn from_authority(
        config: &RuntimeConfig,
        authority: &RecoveryAuthority,
    ) -> Result<Self, String> {
        if config.mcp_session_id.is_empty() {
            return Err(String::from("recovery MCP session identity is empty"));
        }
        Ok(Self {
            instance_id: authority.instance_id.clone(),
            lease_id: authority.lease_id.clone(),
            lease_epoch: authority.lease_epoch,
            mcp_session_id: config.mcp_session_id.clone(),
            current_fence: authority.current_fence.clone(),
        })
    }

    pub(in super::super) fn original_context_raw(
        authority: &RecoveryAuthority,
    ) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&json!({
            "deployment_id": authority.deployment_id,
            "instance_id": authority.instance_id,
            "instance_incarnation": authority.instance_incarnation,
            "boot_id": authority.boot_id,
            "authority_generation": authority.authority_generation,
            "lease_id": authority.lease_id,
            "lease_epoch": authority.lease_epoch,
        }))
        .map_err(|error| format!("cannot encode original recovery context: {error}"))
    }

    pub(in super::super) fn operation_ref(
        operation: &sts2_harness::StoredOperation,
    ) -> Result<Value, String> {
        let original_context_raw = operation
            .intent
            .original_context_raw
            .as_deref()
            .ok_or_else(|| {
                String::from(
                    "durable operation has no immutable original recovery context evidence",
                )
            })?;
        let original_context: Value =
            serde_json::from_slice(original_context_raw).map_err(|_| {
                String::from("durable operation original recovery context is malformed")
            })?;
        let catalog_digest = operation.intent.catalog_digest.as_deref().ok_or_else(|| {
            String::from("durable operation has no original legal-action catalog digest")
        })?;
        let catalog_raw = operation.intent.catalog_raw.as_deref().ok_or_else(|| {
            String::from("durable operation has no retained legal-action catalog bytes")
        })?;
        if !valid_uuid_v4(&operation.intent.operation_id)
            || !valid_digest(&operation.intent.payload_digest)
            || !valid_digest(catalog_digest)
            || format!("{:x}", sha2::Sha256::digest(catalog_raw)) != catalog_digest
            || !valid_uuid(&operation.intent.state_id)
        {
            return Err(String::from(
                "durable operation boundary is incompatible with the recovery sideband",
            ));
        }
        Ok(json!({
            "operation_id": operation.intent.operation_id,
            "payload_digest": operation.intent.payload_digest,
            "original_context": original_context,
        }))
    }

    pub(super) fn reconcile_payload(
        &self,
        operation: &sts2_harness::StoredOperation,
    ) -> Result<Value, String> {
        Ok(json!({
            "operation": Self::operation_ref(operation)?,
            "strategy": "receipt_lookup",
            "current_fence": self.current_fence,
        }))
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_uuid_v4(value: &str) -> bool {
    valid_uuid(value) && value.as_bytes().get(14) == Some(&b'4')
}
