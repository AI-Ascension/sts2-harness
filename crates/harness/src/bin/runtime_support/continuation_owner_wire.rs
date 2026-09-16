// SPDX-License-Identifier: MIT

//! Closed-frame validation and fixed route helpers for continuation owner fencing.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::super::super::config::RuntimeConfig;
use super::super::allocation_context::RecoveryAuthority;
use super::OwnerGatewayPort;
use super::{CLAIM_PATH, CONTRACT, LOOKUP_PATH, MAX_SAFE_INTEGER, READ_PATH, SCHEMA_DIGEST};

pub(super) fn read_available_owner<P: OwnerGatewayPort>(
    gateway: &mut P,
    config: &RuntimeConfig,
) -> Result<Value, String> {
    let response = send_frame(
        gateway,
        config,
        READ_PATH,
        "continuation_owner_read",
        "current_owner_request",
        json!({}),
    )?;
    validate_response_frame(
        &response,
        config,
        "current_owner_response",
        "continuation_owner_read",
    )?;
    if response["payload"]["state"] != "available" {
        return Err(String::from(
            "gateway reports no live available owner for selected branch continuation",
        ));
    }
    let owner = response["payload"]["owner"].clone();
    validate_owner_shape(&owner)?;
    if owner["lease_expires_at_millis"]
        .as_u64()
        .unwrap_or_default()
        <= now_millis()?
    {
        return Err(String::from(
            "gateway owner lease is expired; selected branch continuation is refused",
        ));
    }
    Ok(owner)
}

pub(super) fn claim_owner<P: OwnerGatewayPort>(
    gateway: &mut P,
    config: &RuntimeConfig,
    operation_id: &str,
    owner: &Value,
) -> Result<Value, String> {
    send_frame(
        gateway,
        config,
        CLAIM_PATH,
        "continuation_owner_claim",
        "owner_claim_request",
        json!({"operation_id": operation_id, "expected_owner": owner}),
    )
}

pub(super) fn lookup_claim<P: OwnerGatewayPort>(
    gateway: &mut P,
    config: &RuntimeConfig,
    operation_id: &str,
) -> Result<Value, String> {
    send_frame(
        gateway,
        config,
        LOOKUP_PATH,
        "continuation_owner_lookup",
        "owner_claim_lookup_request",
        json!({"operation_id": operation_id}),
    )
}

fn send_frame<P: OwnerGatewayPort>(
    gateway: &mut P,
    config: &RuntimeConfig,
    path: &str,
    capability: &str,
    kind: &str,
    payload: Value,
) -> Result<Value, String> {
    let request_id = Uuid::new_v4().to_string();
    let correlation_id = Uuid::new_v4().to_string();
    let frame = json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": request_id,
        "correlation_id": correlation_id,
        "actor": {"principal_id": config.caller_id, "role": "harness"},
        "auth": {
            "principal_id": config.caller_id,
            "capability": capability,
            "proof": Value::Null
        },
        "kind": kind,
        "payload": payload
    });
    let response = gateway.post(path, capability, &frame)?;
    validate_response_frame(&response, config, response_kind(kind)?, capability)?;
    if response["correlation_id"].as_str() != Some(correlation_id.as_str()) {
        return Err(String::from(
            "gateway continuation-owner response correlation did not match",
        ));
    }
    Ok(response)
}

fn response_kind(request_kind: &str) -> Result<&'static str, String> {
    match request_kind {
        "current_owner_request" => Ok("current_owner_response"),
        "owner_claim_request" => Ok("owner_claim_response"),
        "owner_claim_lookup_request" => Ok("owner_claim_lookup_response"),
        _ => Err(String::from("unsupported continuation-owner request kind")),
    }
}

pub(super) fn validate_response_frame(
    frame: &Value,
    config: &RuntimeConfig,
    expected_kind: &str,
    capability: &str,
) -> Result<(), String> {
    let object = frame
        .as_object()
        .ok_or_else(|| String::from("gateway continuation-owner response is not an object"))?;
    let expected = [
        "contract",
        "schema_digest",
        "message_id",
        "correlation_id",
        "actor",
        "auth",
        "kind",
        "payload",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(String::from(
            "gateway continuation-owner response has unexpected fields",
        ));
    }
    let actor = exact_object(&frame["actor"], &["principal_id", "role"], "response actor")?;
    let auth = exact_object(
        &frame["auth"],
        &["principal_id", "capability", "proof"],
        "response auth",
    )?;
    if frame["contract"] != CONTRACT
        || frame["schema_digest"] != SCHEMA_DIGEST
        || frame["kind"] != expected_kind
        || !valid_uuid_v4(frame["message_id"].as_str())
        || !valid_uuid_v4(frame["correlation_id"].as_str())
        || frame["actor"]["role"] != "gateway"
        || frame["actor"]["principal_id"] != config.caller_id
        || frame["auth"]["principal_id"] != config.caller_id
        || frame["auth"]["capability"] != capability
        || !frame["auth"]["proof"].is_null()
        || actor.len() != 2
        || auth.len() != 3
    {
        return Err(String::from(
            "gateway continuation-owner response did not match its closed contract",
        ));
    }
    validate_response_payload(frame, expected_kind)?;
    Ok(())
}

pub(super) fn validate_claim_response(
    frame: &Value,
    config: &RuntimeConfig,
    operation_id: &str,
    expected_owner: &Value,
) -> Result<(), String> {
    validate_response_frame(
        frame,
        config,
        "owner_claim_response",
        "continuation_owner_claim",
    )?;
    let claim = &frame["payload"]["claim"];
    if frame["payload"]["result"]
        .as_str()
        .is_none_or(|result| !matches!(result, "CLAIMED" | "DUPLICATE"))
        || claim["operation_id"].as_str() != Some(operation_id)
        || claim["owner"] != *expected_owner
    {
        return Err(String::from(
            "gateway returned a different selected-branch owner claim",
        ));
    }
    Ok(())
}

include!("continuation_owner_wire_binding.rs");

pub(super) fn validate_owner_shape(owner: &Value) -> Result<(), String> {
    let object = owner
        .as_object()
        .ok_or_else(|| String::from("gateway current owner omitted its owner tuple"))?;
    let required = [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "host_fence_id",
        "host_fence_generation",
        "lease_id",
        "lease_epoch",
        "session_id",
        "lease_expires_at_millis",
    ];
    if object.len() != required.len() || required.iter().any(|field| !object.contains_key(*field)) {
        return Err(String::from(
            "gateway current owner tuple has unexpected fields",
        ));
    }
    for field in [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "host_fence_id",
        "lease_id",
        "session_id",
    ] {
        if owner[field]
            .as_str()
            .is_none_or(|value| value.is_empty() || value.len() > 512)
        {
            return Err(String::from(
                "gateway current owner contains an invalid identity",
            ));
        }
    }
    for field in [
        "authority_generation",
        "host_fence_generation",
        "lease_epoch",
        "lease_expires_at_millis",
    ] {
        if owner[field]
            .as_u64()
            .is_none_or(|value| value > MAX_SAFE_INTEGER)
        {
            return Err(String::from(
                "gateway current owner contains an invalid fence counter",
            ));
        }
    }
    Ok(())
}

include!("continuation_owner_wire_payload.rs");

pub(super) fn valid_uuid_v4(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        Uuid::parse_str(value).ok().is_some_and(|uuid| {
            uuid.hyphenated().to_string() == value
                && uuid.get_variant() == uuid::Variant::RFC4122
                && uuid.get_version_num() == 4
        })
    })
}

pub(super) fn is_lower_sha256(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub(super) fn now_millis() -> Result<u64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| String::from("system clock is before Unix epoch"))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| String::from("system clock exceeds the owner protocol range"))
}
