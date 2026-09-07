// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::decode_recovery_action;
use super::values::{positive_u53, strict_timestamp, u53};
use super::{enum_value, exact, object, string, valid_digest, valid_uuid, valid_uuid_v4};

pub(super) fn validate_operation_or_null(value: Option<&Value>, label: &str) -> Result<(), String> {
    match value {
        Some(Value::Null) => Ok(()),
        Some(value) => validate_operation(value),
        None => Err(format!("{label} is missing")),
    }
}

fn validate_operation(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery operation")?;
    exact(
        object,
        &[
            "operation_id",
            "state",
            "payload_digest",
            "original_context",
            "expected_boundary",
            "action",
            "ticket",
            "witness",
            "uncertainty_reason",
            "created_at",
            "updated_at",
        ],
        "recovery operation",
    )?;
    if !valid_uuid_v4(string(object, "operation_id")?) {
        return Err(String::from("recovery operation ID is invalid"));
    }
    enum_value(
        string(object, "state")?,
        &[
            "INTENT_RECORDED",
            "MAY_HAVE_BEEN_DISPATCHED",
            "ACCEPTED",
            "SETTLED",
            "REJECTED",
            "UNKNOWN",
            "RECONCILED",
        ],
        "recovery operation state",
    )?;
    if !valid_digest(string(object, "payload_digest")?) {
        return Err(String::from("recovery operation digest is invalid"));
    }
    validate_original_context(
        object
            .get("original_context")
            .ok_or("recovery operation original context is missing")?,
    )?;
    validate_boundary(
        object
            .get("expected_boundary")
            .ok_or("recovery operation expected boundary is missing")?,
    )?;
    validate_action(
        object
            .get("action")
            .ok_or("recovery operation action is missing")?,
    )?;
    match object.get("ticket") {
        Some(Value::Null) => {}
        Some(value) => validate_ticket(value)?,
        None => return Err(String::from("recovery operation ticket is missing")),
    }
    match object.get("witness") {
        Some(Value::Null) => {}
        Some(value) => validate_witness(value)?,
        None => return Err(String::from("recovery operation witness is missing")),
    }
    match object.get("uncertainty_reason") {
        Some(Value::Null) => {}
        Some(Value::String(value)) => enum_value(
            value,
            &[
                "transport_lost",
                "timeout",
                "gateway_crash",
                "host_crash",
                "receipt_missing",
                "authority_rotated",
            ],
            "recovery uncertainty reason",
        )?,
        _ => return Err(String::from("recovery uncertainty reason is invalid")),
    }
    for field in ["created_at", "updated_at"] {
        if !strict_timestamp(string(object, field)?) {
            return Err(format!(
                "recovery operation {field} is not a strict UTC timestamp"
            ));
        }
    }
    Ok(())
}

fn validate_original_context(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery original context")?;
    exact(
        object,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "lease_id",
            "lease_epoch",
        ],
        "recovery original context",
    )?;
    for field in ["deployment_id", "instance_id"] {
        if !valid_uuid(string(object, field)?) {
            return Err(format!("recovery original context {field} is invalid"));
        }
    }
    for field in ["instance_incarnation", "boot_id", "lease_id"] {
        if !valid_uuid_v4(string(object, field)?) {
            return Err(format!("recovery original context {field} is invalid"));
        }
    }
    for field in ["authority_generation", "lease_epoch"] {
        if positive_u53(
            object
                .get(field)
                .ok_or_else(|| format!("recovery original context {field} is missing"))?,
        )
        .is_none()
        {
            return Err(format!("recovery original context {field} is invalid"));
        }
    }
    Ok(())
}

fn validate_boundary(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery expected boundary")?;
    exact(
        object,
        &["state_id", "generation", "catalog_digest"],
        "recovery expected boundary",
    )?;
    if !valid_uuid(string(object, "state_id")?) {
        return Err(String::from(
            "recovery expected boundary state ID is invalid",
        ));
    }
    if !u53(object
        .get("generation")
        .ok_or("recovery expected boundary generation is missing")?)
    {
        return Err(String::from(
            "recovery expected boundary generation is invalid",
        ));
    }
    if !valid_digest(string(object, "catalog_digest")?) {
        return Err(String::from("recovery catalog digest is invalid"));
    }
    Ok(())
}

fn validate_action(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery action")?;
    exact(
        object,
        &["schema_digest", "canonical_json_b64", "payload_digest"],
        "recovery action",
    )?;
    if !valid_digest(string(object, "schema_digest")?)
        || !valid_digest(string(object, "payload_digest")?)
    {
        return Err(String::from("recovery action digest is invalid"));
    }
    let encoded = string(object, "canonical_json_b64")?;
    if decode_recovery_action(encoded).is_none() {
        return Err(String::from(
            "recovery canonical action encoding is invalid",
        ));
    }
    Ok(())
}

fn validate_ticket(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery ticket")?;
    exact(
        object,
        &[
            "ticket_id",
            "operation_id",
            "payload_digest",
            "boot_id",
            "instance_incarnation",
            "lease_epoch",
            "host_fence_id",
            "state",
            "issued_at",
            "expires_at",
        ],
        "recovery ticket",
    )?;
    for field in [
        "ticket_id",
        "operation_id",
        "boot_id",
        "instance_incarnation",
        "host_fence_id",
    ] {
        if !valid_uuid_v4(string(object, field)?) {
            return Err(format!("recovery ticket {field} is invalid"));
        }
    }
    if !valid_digest(string(object, "payload_digest")?)
        || positive_u53(
            object
                .get("lease_epoch")
                .ok_or("recovery ticket lease epoch is missing")?,
        )
        .is_none()
    {
        return Err(String::from("recovery ticket identity is invalid"));
    }
    enum_value(
        string(object, "state")?,
        &[
            "ISSUED",
            "ADMITTED",
            "EXECUTING",
            "EFFECT_WITNESS_RECORDED",
            "SETTLED",
            "REJECTED",
            "UNKNOWN",
        ],
        "recovery ticket state",
    )?;
    for field in ["issued_at", "expires_at"] {
        if !strict_timestamp(string(object, field)?) {
            return Err(format!(
                "recovery ticket {field} is not a strict UTC timestamp"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_witness(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery witness")?;
    exact(
        object,
        &[
            "witness_id",
            "operation_id",
            "payload_digest",
            "boot_id",
            "instance_incarnation",
            "host_fence_id",
            "source",
            "state_id",
            "generation",
            "effect_digest",
            "observed_at",
        ],
        "recovery witness",
    )?;
    for field in [
        "witness_id",
        "operation_id",
        "boot_id",
        "instance_incarnation",
        "host_fence_id",
    ] {
        if !valid_uuid_v4(string(object, field)?) {
            return Err(format!("recovery witness {field} is invalid"));
        }
    }
    if !valid_uuid(string(object, "state_id")?)
        || !valid_digest(string(object, "payload_digest")?)
        || !valid_digest(string(object, "effect_digest")?)
        || !u53(object
            .get("generation")
            .ok_or("recovery witness generation is missing")?)
    {
        return Err(String::from("recovery witness identity is invalid"));
    }
    enum_value(
        string(object, "source")?,
        &[
            "host_game_thread",
            "host_receipt",
            "authoritative_reobserve",
        ],
        "recovery witness source",
    )?;
    if !strict_timestamp(string(object, "observed_at")?) {
        return Err(String::from("recovery witness observed_at is invalid"));
    }
    Ok(())
}
