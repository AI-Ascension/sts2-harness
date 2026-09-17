// SPDX-License-Identifier: MIT
//! Validation for the additive live-observation bootstrap consumed by the Harness.
//!
//! The gateway and game mod remain the protocol authorities.  This module only
//! checks the bounded, authenticated transcript before a native snapshot can
//! become the live query binding.

use serde_json::{Value, json};

pub const PROFILE: &str = "game-information-live-observation-bootstrap-v1";
pub const SCHEMA_DIGEST: &str = "6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2";
pub const MAX_VISIBLE_ENTITIES: usize = 64;
pub const MAX_ITEM_BYTES: usize = 65_536;
pub const MAX_MESSAGE_BYTES: usize = 262_144;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootstrapError {
    Invalid,
    Scope,
    Bounds,
    Ambiguous,
    Unavailable,
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "live observation bootstrap: {self:?}")
    }
}

impl std::error::Error for BootstrapError {}

pub fn request(
    correlation_id: &str,
    scope: Value,
    definition_ref: Value,
    instance_ref: Option<Value>,
) -> Value {
    json!({
        "protocol_version": PROFILE,
        "schema_digest": SCHEMA_DIGEST,
        "provenance": {
            "artifact": "sts2-protocol/game-information-live-observation-bootstrap-v1",
            "source": "schemas/game-information-live-observation-bootstrap-v1.schema.json",
            "generator": "hand-authored"
        },
        "correlation_id": correlation_id,
        "kind": "bootstrap_request",
        "selector": {"definition_ref": definition_ref, "instance_ref": instance_ref},
        "scope": scope,
        "limits": {
            "max_visible_entities": MAX_VISIBLE_ENTITIES,
            "max_item_bytes": MAX_ITEM_BYTES,
            "max_message_bytes": MAX_MESSAGE_BYTES
        },
        "parent_observation": null,
        "visible_entities": null,
        "owner_provenance": null,
        "error": null
    })
}

pub fn validate_request(value: &Value) -> Result<(), BootstrapError> {
    if serde_json::to_vec(value)
        .map_err(|_| BootstrapError::Invalid)?
        .len()
        > MAX_MESSAGE_BYTES
    {
        return Err(BootstrapError::Bounds);
    }
    let envelope = value.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        envelope,
        &[
            "protocol_version",
            "schema_digest",
            "provenance",
            "correlation_id",
            "kind",
            "selector",
            "scope",
            "limits",
            "parent_observation",
            "visible_entities",
            "owner_provenance",
            "error",
        ],
    ) || value["protocol_version"] != PROFILE
        || value["schema_digest"] != SCHEMA_DIGEST
        || value["kind"] != "bootstrap_request"
        || value["provenance"]["artifact"]
            != "sts2-protocol/game-information-live-observation-bootstrap-v1"
        || value["provenance"]["source"]
            != "schemas/game-information-live-observation-bootstrap-v1.schema.json"
        || value["provenance"]["generator"] != "hand-authored"
        || !valid_identity(value["correlation_id"].as_str())
        || value["parent_observation"] != Value::Null
        || value["visible_entities"] != Value::Null
        || value["owner_provenance"] != Value::Null
        || value["error"] != Value::Null
    {
        return Err(BootstrapError::Invalid);
    }
    if !exact_keys(
        value["provenance"]
            .as_object()
            .ok_or(BootstrapError::Invalid)?,
        &["artifact", "source", "generator"],
    ) {
        return Err(BootstrapError::Invalid);
    }
    validate_scope(&value["scope"])?;
    validate_limits(&value["limits"])?;
    let selector = value["selector"]
        .as_object()
        .ok_or(BootstrapError::Invalid)?;
    if !exact_keys(selector, &["definition_ref", "instance_ref"]) {
        return Err(BootstrapError::Invalid);
    }
    validate_definition(&selector["definition_ref"])?;
    if !selector["instance_ref"].is_null() {
        validate_instance(&selector["instance_ref"])?;
    }
    Ok(())
}

/// Validates a response and returns the exact selected native snapshot reference.
/// The response selector must echo the request selector exactly and every
/// visible entity must carry the selected definition inside the attested
/// scope; a foreign manifest or occurrence fails closed instead of being
/// skipped. An omitted occurrence is accepted only when the definition
/// resolves to one visible entity; no arbitrary item is selected from an
/// ambiguous response.
pub fn select_snapshot(request: &Value, response: &Value) -> Result<Value, BootstrapError> {
    validate_request(request)?;
    if serde_json::to_vec(response)
        .map_err(|_| BootstrapError::Invalid)?
        .len()
        > MAX_MESSAGE_BYTES
    {
        return Err(BootstrapError::Bounds);
    }
    let envelope = response.as_object().ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        envelope,
        &[
            "protocol_version",
            "schema_digest",
            "provenance",
            "correlation_id",
            "kind",
            "selector",
            "scope",
            "limits",
            "parent_observation",
            "visible_entities",
            "owner_provenance",
            "error",
        ],
    ) || response["protocol_version"] != PROFILE
        || response["schema_digest"] != SCHEMA_DIGEST
        || response["kind"] != "bootstrap_response"
        || response["correlation_id"] != request["correlation_id"]
        || response["provenance"]["artifact"]
            != "sts2-protocol/game-information-live-observation-bootstrap-v1"
        || response["provenance"]["source"]
            != "schemas/game-information-live-observation-bootstrap-v1.schema.json"
        || response["provenance"]["generator"] != "hand-authored"
        || response["error"] != Value::Null
    {
        return Err(BootstrapError::Invalid);
    }
    if !exact_keys(
        response["provenance"]
            .as_object()
            .ok_or(BootstrapError::Invalid)?,
        &["artifact", "source", "generator"],
    ) {
        return Err(BootstrapError::Invalid);
    }
    validate_scope(&response["scope"])?;
    if response["scope"] != request["scope"] {
        return Err(BootstrapError::Scope);
    }
    validate_limits(&response["limits"])?;
    if response["limits"]["max_message_bytes"].as_u64() != Some(MAX_MESSAGE_BYTES as u64) {
        return Err(BootstrapError::Bounds);
    }
    validate_owner_provenance(&response["owner_provenance"])?;
    let request_selector = request["selector"]
        .as_object()
        .ok_or(BootstrapError::Invalid)?;
    let response_selector = response["selector"]
        .as_object()
        .ok_or(BootstrapError::Invalid)?;
    if !exact_keys(request_selector, &["definition_ref", "instance_ref"])
        || !exact_keys(response_selector, &["definition_ref", "instance_ref"])
    {
        return Err(BootstrapError::Invalid);
    }
    if response_selector["definition_ref"] != request_selector["definition_ref"] {
        return Err(BootstrapError::Scope);
    }
    if !response_selector["instance_ref"].is_null() {
        validate_instance(&response_selector["instance_ref"])?;
    }
    if response_selector["instance_ref"] != request_selector["instance_ref"] {
        return Err(BootstrapError::Scope);
    }
    let parent = response["parent_observation"]
        .as_object()
        .ok_or(BootstrapError::Invalid)?;
    if !exact_keys(
        parent,
        &["instance_ref", "snapshot_ref", "state_generation"],
    ) {
        return Err(BootstrapError::Invalid);
    }
    validate_instance(&parent["instance_ref"])?;
    let parent_snapshot = parent["snapshot_ref"]
        .as_object()
        .ok_or(BootstrapError::Invalid)?;
    validate_snapshot(parent_snapshot)?;
    let generation = parent["state_generation"]
        .as_u64()
        .ok_or(BootstrapError::Invalid)?;
    if parent_snapshot["state_generation"].as_u64() != Some(generation)
        || parent_snapshot["instance_ref"] != parent["instance_ref"]
    {
        return Err(BootstrapError::Scope);
    }
    let visible = response["visible_entities"]
        .as_array()
        .ok_or(BootstrapError::Invalid)?;
    if visible.is_empty() || visible.len() > MAX_VISIBLE_ENTITIES {
        return Err(BootstrapError::Bounds);
    }
    let scope_instance = response["scope"]["instance_id"].as_str();
    let scope_run = response["scope"]["run_id"].as_str();
    let scope_manifest = response["scope"]["content_manifest_id"].as_str();
    let mut candidates = Vec::new();
    for entity in visible {
        let object = entity.as_object().ok_or(BootstrapError::Invalid)?;
        if !exact_keys(object, &["instance_ref", "snapshot_ref", "definition_ref"]) {
            return Err(BootstrapError::Invalid);
        }
        validate_instance(&object["instance_ref"])?;
        validate_snapshot(
            object["snapshot_ref"]
                .as_object()
                .ok_or(BootstrapError::Invalid)?,
        )?;
        let instance = &object["instance_ref"];
        if instance["instance_id"].as_str() != scope_instance
            || instance["run_id"].as_str() != scope_run
            || object["snapshot_ref"]["instance_ref"] != *instance
            || object["snapshot_ref"]["state_generation"] != json!(generation)
        {
            return Err(BootstrapError::Scope);
        }
        let definition = object["definition_ref"]
            .as_object()
            .ok_or(BootstrapError::Invalid)?;
        validate_definition(&Value::Object(definition.clone()))?;
        if definition["content_manifest_id"].as_str() != scope_manifest
            || Value::Object(definition.clone()) != request_selector["definition_ref"]
        {
            return Err(BootstrapError::Scope);
        }
        let occurrence = request_selector["instance_ref"].as_object();
        if occurrence.is_some_and(|expected| &Value::Object(expected.clone()) != instance) {
            continue;
        }
        candidates.push(object["snapshot_ref"].clone());
    }
    if candidates.is_empty() {
        return Err(BootstrapError::Unavailable);
    }
    if candidates.len() != 1 {
        return Err(BootstrapError::Ambiguous);
    }
    let selected = candidates.pop().ok_or(BootstrapError::Invalid)?;
    if !response_selector["instance_ref"].is_null()
        && selected["instance_ref"] != response_selector["instance_ref"]
    {
        return Err(BootstrapError::Scope);
    }
    Ok(selected)
}

include!("game_information_bootstrap_validation.rs");

#[cfg(test)]
#[path = "game_information_bootstrap_tests.rs"]
mod tests;
