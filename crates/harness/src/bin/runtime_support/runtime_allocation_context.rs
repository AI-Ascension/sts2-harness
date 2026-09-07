// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::super::config::RuntimeConfig;

#[path = "runtime_allocation_timestamp.rs"]
mod timestamp;

pub(super) const ALLOCATION_SCHEMA_DIGEST: &str =
    "ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b";

const ALLOCATION_CONTRACT: &str = "watchdog-runtime-allocation-v1";
const CONTEXT_KEYS: &[&str] = &[
    "deployment_id",
    "instance_id",
    "instance_incarnation",
    "boot_id",
    "authority_generation",
    "lease_id",
    "lease_epoch",
];
const FENCE_KEYS: &[&str] = &[
    "host_fence_id",
    "deployment_id",
    "instance_id",
    "instance_incarnation",
    "boot_id",
    "authority_generation",
    "fence_generation",
    "created_at",
];
const AUTHORITY_KEYS: &[&str] = &["contract", "schema_digest", "context", "current_fence"];
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecoveryAuthority {
    pub(super) deployment_id: String,
    pub(super) instance_id: String,
    pub(super) instance_incarnation: String,
    pub(super) boot_id: String,
    pub(super) authority_generation: u64,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) current_fence: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ValidatedAllocation {
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) recovery_authority: Option<RecoveryAuthority>,
}

impl ValidatedAllocation {
    pub(super) fn apply_current_lease(&self, config: &mut RuntimeConfig) {
        config.lease_id.clone_from(&self.lease_id);
        config.lease_epoch = self.lease_epoch;
    }
}

pub(super) fn validate(
    value: &Value,
    config: &RuntimeConfig,
) -> Result<ValidatedAllocation, String> {
    if value.get("status").and_then(Value::as_str) != Some("allocated") {
        return Err(String::from("gateway allocation status was not allocated"));
    }
    for (key, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("caller_id", config.caller_id.as_str()),
        ("session_id", config.session_id.as_str()),
    ] {
        if value.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(format!("gateway allocation returned unexpected {key}"));
        }
    }
    let lease_id = value
        .get("lease_id")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("gateway allocation omitted lease_id"))?;
    let lease_epoch = value
        .get("lease_epoch")
        .and_then(Value::as_u64)
        .ok_or_else(|| String::from("gateway allocation omitted lease_epoch"))?;
    if lease_id.is_empty() || lease_epoch == 0 || lease_epoch > MAX_SAFE_INTEGER {
        return Err(String::from("gateway allocation returned an invalid lease"));
    }

    match value.get("recovery_authority") {
        Some(authority) => {
            let authority = parse_authority(authority, config.instance_id.as_str())?;
            if authority.lease_id != lease_id || authority.lease_epoch != lease_epoch {
                return Err(String::from(
                    "gateway allocation authority does not match its lease",
                ));
            }
            Ok(ValidatedAllocation {
                lease_id: authority.lease_id.clone(),
                lease_epoch: authority.lease_epoch,
                recovery_authority: Some(authority),
            })
        }
        None if config.recovery_value("STS2_RECOVERY_TOKEN").is_some() => Err(String::from(
            "recovery allocation omitted recovery_authority",
        )),
        None if lease_id != config.lease_id || lease_epoch != config.lease_epoch => {
            Err(String::from("gateway allocation returned unexpected lease"))
        }
        None => Ok(ValidatedAllocation {
            lease_id: lease_id.to_owned(),
            lease_epoch,
            recovery_authority: None,
        }),
    }
}

fn parse_authority(value: &Value, expected_instance_id: &str) -> Result<RecoveryAuthority, String> {
    let authority = exact_object(value, AUTHORITY_KEYS, "recovery_authority")?;
    if authority.get("contract").and_then(Value::as_str) != Some(ALLOCATION_CONTRACT)
        || authority.get("schema_digest").and_then(Value::as_str) != Some(ALLOCATION_SCHEMA_DIGEST)
    {
        return Err(String::from(
            "recovery allocation authority contract is invalid",
        ));
    }
    let context = exact_object(
        authority
            .get("context")
            .ok_or_else(|| String::from("recovery allocation context is missing"))?,
        CONTEXT_KEYS,
        "recovery allocation context",
    )?;
    let deployment_id = required_uuid(context, "deployment_id", false)?;
    let instance_id = required_uuid(context, "instance_id", false)?;
    if instance_id != expected_instance_id {
        return Err(String::from(
            "recovery allocation context instance does not match the request",
        ));
    }
    let instance_incarnation = required_uuid(context, "instance_incarnation", true)?;
    let boot_id = required_uuid(context, "boot_id", true)?;
    let authority_generation = required_u53(context, "authority_generation")?;
    let lease_id = required_uuid(context, "lease_id", true)?;
    let lease_epoch = required_u53(context, "lease_epoch")?;

    let fence = authority
        .get("current_fence")
        .ok_or_else(|| String::from("recovery allocation current fence is missing"))?;
    validate_fence(
        fence,
        &deployment_id,
        &instance_id,
        &instance_incarnation,
        &boot_id,
        authority_generation,
    )?;
    Ok(RecoveryAuthority {
        deployment_id,
        instance_id,
        instance_incarnation,
        boot_id,
        authority_generation,
        lease_id,
        lease_epoch,
        current_fence: fence.clone(),
    })
}

fn validate_fence(
    value: &Value,
    deployment_id: &str,
    instance_id: &str,
    instance_incarnation: &str,
    boot_id: &str,
    authority_generation: u64,
) -> Result<(), String> {
    let fence = exact_object(value, FENCE_KEYS, "recovery allocation current fence")?;
    required_uuid(fence, "host_fence_id", true)?;
    if fence.get("deployment_id").and_then(Value::as_str) != Some(deployment_id)
        || fence.get("instance_id").and_then(Value::as_str) != Some(instance_id)
        || fence.get("instance_incarnation").and_then(Value::as_str) != Some(instance_incarnation)
        || fence.get("boot_id").and_then(Value::as_str) != Some(boot_id)
        || fence.get("authority_generation").and_then(Value::as_u64) != Some(authority_generation)
    {
        return Err(String::from(
            "recovery allocation current fence does not match its context",
        ));
    }
    required_u53(fence, "fence_generation")?;
    let created_at = fence
        .get("created_at")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("recovery allocation current fence timestamp is invalid"))?;
    if !timestamp::valid(created_at) {
        return Err(String::from(
            "recovery allocation current fence timestamp is invalid",
        ));
    }
    Ok(())
}

fn exact_object<'a>(
    value: &'a Value,
    expected: &[&str],
    name: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{name} is not an object"))?;
    if object.len() != expected.len()
        || object.keys().any(|key| !expected.contains(&key.as_str()))
        || expected.iter().any(|key| !object.contains_key(*key))
    {
        return Err(format!("{name} has unexpected fields"));
    }
    Ok(object)
}

fn required_uuid(object: &Map<String, Value>, key: &str, v4: bool) -> Result<String, String> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| {
            if v4 {
                valid_uuid_v4(value)
            } else {
                valid_uuid(value)
            }
        })
        .ok_or_else(|| format!("recovery allocation {key} is invalid"))?;
    Ok(value.to_owned())
}

fn required_u53(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    let value = object
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0 && *value <= MAX_SAFE_INTEGER)
        .ok_or_else(|| format!("recovery allocation {key} is invalid"))?;
    Ok(value)
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

#[cfg(test)]
#[path = "runtime_allocation_context_tests.rs"]
mod tests;
