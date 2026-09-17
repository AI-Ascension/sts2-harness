// SPDX-License-Identifier: MIT
use serde_json::Map;

fn valid_identity(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    })
}

fn validate_scope(value: &Value) -> Result<(), BootstrapError> {
    let object = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        object,
        &[
            "instance_id",
            "run_id",
            "authority_epoch",
            "content_manifest_id",
            "locale",
        ],
    ) {
        return Err(BootstrapError::Invalid);
    }
    for key in ["instance_id", "run_id", "content_manifest_id"] {
        if !valid_identity(object.get(key).and_then(Value::as_str)) {
            return Err(BootstrapError::Invalid);
        }
    }
    if !object
        .get("locale")
        .and_then(Value::as_str)
        .is_some_and(|locale| locale.len() >= 2 && locale.len() <= 35)
        || object
            .get("authority_epoch")
            .and_then(Value::as_u64)
            .is_none_or(|epoch| epoch == 0)
    {
        return Err(BootstrapError::Invalid);
    }
    Ok(())
}

fn validate_limits(value: &Value) -> Result<(), BootstrapError> {
    let object = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(object, &["max_visible_entities", "max_item_bytes", "max_message_bytes"]) {
        return Err(BootstrapError::Invalid);
    }
    let visible = object.get("max_visible_entities").and_then(Value::as_u64);
    let item = object.get("max_item_bytes").and_then(Value::as_u64);
    let message = object.get("max_message_bytes").and_then(Value::as_u64);
    if !visible.is_some_and(|v| (1..=MAX_VISIBLE_ENTITIES as u64).contains(&v))
        || !item.is_some_and(|v| (1..=MAX_ITEM_BYTES as u64).contains(&v))
        || !message.is_some_and(|v| (1..=MAX_MESSAGE_BYTES as u64).contains(&v))
    {
        return Err(BootstrapError::Bounds);
    }
    Ok(())
}

fn validate_definition(value: &Value) -> Result<(), BootstrapError> {
    let object = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        object,
        &["content_manifest_id", "entity_kind", "namespaced_id", "variant"],
    ) {
        return Err(BootstrapError::Invalid);
    }
    if !valid_identity(object.get("content_manifest_id").and_then(Value::as_str))
        || !valid_identity(object.get("namespaced_id").and_then(Value::as_str))
        || !matches!(
            object.get("entity_kind").and_then(Value::as_str),
            Some("card" | "character" | "enemy" | "event" | "map_node" | "potion" | "power"
                | "relic" | "room" | "status")
        )
        || !object
            .get("variant")
            .is_some_and(|value| value.is_null() || valid_identity(value.as_str()))
    {
        return Err(BootstrapError::Invalid);
    }
    Ok(())
}

fn validate_instance(value: &Value) -> Result<(), BootstrapError> {
    let object = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        object,
        &["instance_id", "run_id", "epoch", "entity_kind", "entity_id"],
    ) {
        return Err(BootstrapError::Invalid);
    }
    if !valid_identity(object.get("instance_id").and_then(Value::as_str))
        || !valid_identity(object.get("run_id").and_then(Value::as_str))
        || !valid_identity(object.get("entity_id").and_then(Value::as_str))
        || object.get("epoch").and_then(Value::as_u64).is_none()
        || !matches!(
            object.get("entity_kind").and_then(Value::as_str),
            Some("card" | "character" | "enemy" | "event" | "map_node" | "potion" | "power"
                | "relic" | "room" | "status")
        )
    {
        return Err(BootstrapError::Invalid);
    }
    Ok(())
}

fn validate_snapshot(value: &Map<String, Value>) -> Result<(), BootstrapError> {
    if !exact_keys(value, &["snapshot_id", "instance_ref", "state_generation"]) {
        return Err(BootstrapError::Invalid);
    }
    if !valid_identity(value.get("snapshot_id").and_then(Value::as_str))
        || value.get("state_generation").and_then(Value::as_u64).is_none()
    {
        return Err(BootstrapError::Invalid);
    }
    validate_instance(value.get("instance_ref").ok_or(BootstrapError::Invalid)?)
}

fn validate_owner_provenance(value: &Value) -> Result<(), BootstrapError> {
    let object = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        object,
        &[
            "native_snapshot_owner",
            "content_manifest_owner",
            "instance_fence_owner",
            "authority_epoch_owner",
            "instance_ref_epoch_owner",
            "transport_lease_epoch_role",
        ],
    ) {
        return Err(BootstrapError::Invalid);
    }
    let expected = [
        ("native_snapshot_owner", "sts2-game-mod"),
        ("content_manifest_owner", "sts2-game-mod"),
        ("instance_fence_owner", "sts2-gateway"),
        ("authority_epoch_owner", "sts2-harness"),
        ("instance_ref_epoch_owner", "sts2-game-mod"),
        ("transport_lease_epoch_role", "fence_only"),
    ];
    if expected
        .iter()
        .any(|(key, value)| object.get(*key).and_then(Value::as_str) != Some(*value))
    {
        return Err(BootstrapError::Scope);
    }
    Ok(())
}

fn exact_keys(object: &Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}
