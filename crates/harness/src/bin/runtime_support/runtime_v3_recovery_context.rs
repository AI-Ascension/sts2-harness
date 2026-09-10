// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::Digest;

use super::super::super::config::RuntimeConfig;
use super::super::allocation_context::RecoveryAuthority;

const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug)]
pub(in super::super) struct RecoveryContext {
    pub(super) instance_id: String,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) mcp_session_id: String,
    original_context: Value,
    current_fence: Value,
}

impl RecoveryContext {
    pub(crate) fn from_authority(
        authority: &RecoveryAuthority,
        config: &RuntimeConfig,
    ) -> Result<Self, String> {
        if authority.instance_id != config.instance_id
            || authority.lease_id != config.lease_id
            || authority.lease_epoch != config.lease_epoch
        {
            return Err(String::from(
                "recovery allocation authority does not match the active runtime lease",
            ));
        }
        validate_fence(
            &authority.current_fence,
            &authority.deployment_id,
            &authority.instance_id,
            &authority.instance_incarnation,
            &authority.boot_id,
            authority.authority_generation,
        )?;
        if config.mcp_session_id.is_empty() {
            return Err(String::from("recovery MCP session identity is empty"));
        }
        Ok(Self {
            instance_id: authority.instance_id.clone(),
            lease_id: authority.lease_id.clone(),
            lease_epoch: authority.lease_epoch,
            mcp_session_id: config.mcp_session_id.clone(),
            original_context: json!({
                "deployment_id": authority.deployment_id,
                "instance_id": authority.instance_id,
                "instance_incarnation": authority.instance_incarnation,
                "boot_id": authority.boot_id,
                "authority_generation": authority.authority_generation,
                "lease_id": authority.lease_id,
                "lease_epoch": authority.lease_epoch,
            }),
            current_fence: authority.current_fence.clone(),
        })
    }

    pub(super) fn from_environment(config: &RuntimeConfig) -> Result<Self, String> {
        let deployment_id = required_recovery_env(config, "STS2_RECOVERY_DEPLOYMENT_ID")?;
        let instance_id = required_recovery_env(config, "STS2_RECOVERY_INSTANCE_ID")?;
        let instance_incarnation = required_recovery_env(config, "STS2_RECOVERY_INSTANCE_INCAR")?;
        let boot_id = required_recovery_env(config, "STS2_RECOVERY_BOOT_ID")?;
        let lease_id = required_recovery_env(config, "STS2_RECOVERY_LEASE_ID")?;
        let authority_generation =
            required_recovery_env(config, "STS2_RECOVERY_AUTHORITY_GENERATION")?
                .parse::<u64>()
                .map_err(|_| String::from("STS2_RECOVERY_AUTHORITY_GENERATION is invalid"))?;
        let lease_epoch = required_recovery_env(config, "STS2_RECOVERY_LEASE_EPOCH")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_RECOVERY_LEASE_EPOCH is invalid"))?;
        if !valid_uuid(&deployment_id)
            || !valid_uuid(&instance_id)
            || !valid_uuid_v4(&instance_incarnation)
            || !valid_uuid_v4(&boot_id)
            || !valid_uuid_v4(&lease_id)
            || !positive_wire_integer(authority_generation)
            || !positive_wire_integer(lease_epoch)
        {
            return Err(String::from(
                "recovery original context has an invalid identity or generation",
            ));
        }
        let current_fence: Value = serde_json::from_str(&required_recovery_env(
            config,
            "STS2_RECOVERY_CURRENT_FENCE_JSON",
        )?)
        .map_err(|_| String::from("STS2_RECOVERY_CURRENT_FENCE_JSON is not valid JSON"))?;
        validate_fence(
            &current_fence,
            &deployment_id,
            &instance_id,
            &instance_incarnation,
            &boot_id,
            authority_generation,
        )?;
        if config.mcp_session_id.is_empty() {
            return Err(String::from("recovery MCP session identity is empty"));
        }
        Ok(Self {
            instance_id: instance_id.clone(),
            lease_id: lease_id.clone(),
            lease_epoch,
            mcp_session_id: config.mcp_session_id.clone(),
            original_context: json!({
                "deployment_id": deployment_id,
                "instance_id": instance_id,
                "instance_incarnation": instance_incarnation,
                "boot_id": boot_id,
                "authority_generation": authority_generation,
                "lease_id": lease_id,
                "lease_epoch": lease_epoch,
            }),
            current_fence,
        })
    }

    pub(super) fn operation_ref(
        &self,
        operation: &sts2_harness::StoredOperation,
    ) -> Result<Value, String> {
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
            "original_context": self.original_context,
        }))
    }

    pub(super) fn reconcile_payload(
        &self,
        operation: &sts2_harness::StoredOperation,
    ) -> Result<Value, String> {
        Ok(json!({
            "operation": self.operation_ref(operation)?,
            "strategy": "receipt_lookup",
            "current_fence": self.current_fence,
        }))
    }
}

include!("runtime_v3_recovery_context_validation.rs");
