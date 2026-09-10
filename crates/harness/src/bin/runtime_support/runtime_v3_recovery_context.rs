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
    pub(crate) fn original_context(&self) -> &Value {
        &self.original_context
    }

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
        let original_context = operation
            .intent
            .original_context
            .as_deref()
            .ok_or_else(|| String::from("durable operation has no original recovery context"))?;
        let original_context: Value = serde_json::from_slice(original_context)
            .map_err(|_| String::from("durable operation original recovery context is invalid"))?;
        validate_original_context(&original_context)?;
        if !valid_uuid_v4(&operation.intent.operation_id)
            || !valid_digest(&operation.intent.payload_digest)
            || !valid_digest(catalog_digest)
            || format!("{:x}", sha2::Sha256::digest(catalog_raw)) != catalog_digest
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
            "operation": self.operation_ref(operation)?,
            "strategy": "receipt_lookup",
            "current_fence": self.current_fence,
        }))
    }
}

include!("runtime_v3_recovery_context_child.rs");

fn validate_original_context(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or_else(|| {
        String::from("durable operation original recovery context is not an object")
    })?;
    let expected = [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "lease_id",
        "lease_epoch",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(String::from(
            "durable operation original recovery context has an invalid shape",
        ));
    }
    for field in ["deployment_id", "instance_id"] {
        if !object
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(valid_uuid)
        {
            return Err(format!(
                "durable operation original recovery context {field} is invalid"
            ));
        }
    }
    for field in ["instance_incarnation", "boot_id", "lease_id"] {
        if !object
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(valid_uuid_v4)
        {
            return Err(format!(
                "durable operation original recovery context {field} is invalid"
            ));
        }
    }
    for field in ["authority_generation", "lease_epoch"] {
        if !object
            .get(field)
            .and_then(Value::as_u64)
            .is_some_and(positive_wire_integer)
        {
            return Err(format!(
                "durable operation original recovery context {field} is invalid"
            ));
        }
    }
    Ok(())
}

include!("runtime_v3_recovery_context_validation.rs");

#[cfg(test)]
mod context_tests {
    use super::*;
    use serde_json::json;
    use sha2::Digest;
    use sts2_harness::{ExecutionLineage, OperationIntent, OperationState, StoredOperation};

    #[test]
    fn operation_reference_accepts_bounded_runtime_state_ids() -> Result<(), String> {
        let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")
            .map_err(|error| error.to_string())?;
        let action_payload = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
        let catalog_raw = br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]"#;
        let payload_digest = format!("{:x}", sha2::Sha256::digest(action_payload));
        let catalog_digest = format!("{:x}", sha2::Sha256::digest(catalog_raw));
        let original_context = serde_json::to_vec(&json!({
            "deployment_id": "33333333-3333-3333-8333-333333333333",
            "instance_id": "44444444-4444-4444-8444-444444444444",
            "instance_incarnation": "55555555-5555-4555-8555-555555555555",
            "boot_id": "66666666-6666-4666-8666-666666666666",
            "authority_generation": 1,
            "lease_id": "77777777-7777-4777-8777-777777777777",
            "lease_epoch": 1
        }))
        .map_err(|error| error.to_string())?;
        let intent = OperationIntent::new_with_action_and_catalog(
            lineage,
            "11111111-1111-4111-8111-111111111111",
            "live:7",
            3,
            "combat.end-turn",
            "end_turn",
            action_payload.to_vec(),
            payload_digest,
            "input-digest",
            Some(catalog_digest),
            Some(catalog_raw.to_vec()),
        )
        .map_err(|error| error.to_string())?
        .with_original_context(original_context)
        .map_err(|error| error.to_string())?;
        let operation = StoredOperation {
            intent,
            state: OperationState::Unknown,
            result_ref: None,
            result_digest: None,
        };
        let context = RecoveryContext {
            instance_id: String::from("44444444-4444-4444-8444-444444444444"),
            lease_id: String::from("77777777-7777-4777-8777-777777777777"),
            lease_epoch: 1,
            mcp_session_id: String::from("mcp-session"),
            original_context: json!({}),
            current_fence: Value::Null,
        };
        let reference = context.operation_ref(&operation)?;
        assert_eq!(reference["operation_id"], operation.intent.operation_id);
        assert_eq!(reference["original_context"]["lease_epoch"], 1);
        Ok(())
    }
}
