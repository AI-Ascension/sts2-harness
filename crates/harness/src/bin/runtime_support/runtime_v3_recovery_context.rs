// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::collections::BTreeSet;

use super::super::super::config::RuntimeConfig;

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
            || authority_generation == 0
            || lease_epoch == 0
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
        if !valid_uuid_v4(&operation.intent.operation_id)
            || !valid_digest(&operation.intent.payload_digest)
            || !valid_digest(catalog_digest)
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

fn required_recovery_env(config: &RuntimeConfig, name: &str) -> Result<String, String> {
    // Values are captured in RuntimeConfig before the process environment is scrubbed for child
    // MCP processes. Falling back to the parent environment preserves the live CLI path while
    // allowing tests and embedded callers to provide an isolated configuration directly.
    config
        .recovery_value(name)
        .map(str::to_owned)
        .or_else(|| std::env::var(name).ok())
        .ok_or_else(|| format!("{name} is required for the recovery sideband"))
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
    valid_uuid(value)
        && value.as_bytes().get(14) == Some(&b'4')
        && value
            .as_bytes()
            .get(19)
            .is_some_and(|byte| matches!(*byte, b'8' | b'9' | b'a' | b'b'))
}

fn validate_fence(
    fence: &Value,
    deployment_id: &str,
    instance_id: &str,
    instance_incarnation: &str,
    boot_id: &str,
    authority_generation: u64,
) -> Result<(), String> {
    let object = fence
        .as_object()
        .ok_or_else(|| String::from("recovery current fence is not an object"))?;
    let expected: BTreeSet<&str> = [
        "host_fence_id",
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "fence_generation",
        "created_at",
    ]
    .into_iter()
    .collect();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || object.get("deployment_id").and_then(Value::as_str) != Some(deployment_id)
        || object.get("instance_id").and_then(Value::as_str) != Some(instance_id)
        || object.get("instance_incarnation").and_then(Value::as_str) != Some(instance_incarnation)
        || object.get("boot_id").and_then(Value::as_str) != Some(boot_id)
        || object.get("authority_generation").and_then(Value::as_u64) != Some(authority_generation)
        || !object
            .get("host_fence_id")
            .and_then(Value::as_str)
            .is_some_and(valid_uuid_v4)
        || object.get("fence_generation").and_then(Value::as_u64) == Some(0)
        || !object
            .get("created_at")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= 64)
    {
        return Err(String::from("recovery current fence is invalid"));
    }
    Ok(())
}
