// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{MAX_GENERATION, MapError, RUNTIME_MAP_PROFILE, RUNTIME_MAP_SCHEMA_DIGEST};

pub(super) fn validate(object: &Map<String, Value>) -> Result<(), MapError> {
    let expected = [
        "protocol_version",
        "schema_digest",
        "provenance",
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "lease_epoch",
        "generation",
        "kind",
        "snapshot",
        "timeout",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(MapError::InvalidEnvelope);
    }
    if object.get("protocol_version").and_then(Value::as_str) != Some(RUNTIME_MAP_PROFILE)
        || object.get("schema_digest").and_then(Value::as_str) != Some(RUNTIME_MAP_SCHEMA_DIGEST)
        || object.get("kind").and_then(Value::as_str) != Some("snapshot_response")
    {
        return Err(MapError::UnsupportedVersion);
    }
    for key in ["correlation_id", "instance_id", "session_id", "lease_id"] {
        if !object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(super::valid_identity)
        {
            return Err(MapError::InvalidIdentity);
        }
    }
    for key in ["lease_epoch", "generation"] {
        if object
            .get(key)
            .and_then(Value::as_u64)
            .is_none_or(|value| value > MAX_GENERATION)
        {
            return Err(MapError::InvalidGeneration);
        }
    }
    let provenance = object
        .get("provenance")
        .and_then(Value::as_object)
        .ok_or(MapError::InvalidEnvelope)?;
    if provenance.len() != 3
        || provenance.get("artifact").and_then(Value::as_str)
            != Some("sts2-protocol/runtime-map-v1")
        || provenance.get("source").and_then(Value::as_str)
            != Some("schemas/runtime-map-v1.schema.json")
        || provenance.get("generator").and_then(Value::as_str) != Some("hand-authored")
    {
        return Err(MapError::UnsupportedVersion);
    }
    let timeout = object.get("timeout").ok_or(MapError::InvalidEnvelope)?;
    if !timeout.is_null() {
        let timeout = timeout.as_object().ok_or(MapError::InvalidEnvelope)?;
        if timeout.len() != 2
            || timeout
                .get("timeout_millis")
                .and_then(Value::as_u64)
                .is_none_or(|value| !(1..=120_000).contains(&value))
            || timeout
                .get("elapsed_millis")
                .and_then(Value::as_u64)
                .is_none_or(|value| value > timeout["timeout_millis"].as_u64().unwrap_or(0))
        {
            return Err(MapError::InvalidEnvelope);
        }
    }
    if object
        .get("snapshot")
        .and_then(Value::as_object)
        .and_then(|snapshot| snapshot.get("generation"))
        .and_then(Value::as_u64)
        != object.get("generation").and_then(Value::as_u64)
    {
        return Err(MapError::InvalidGeneration);
    }
    Ok(())
}
