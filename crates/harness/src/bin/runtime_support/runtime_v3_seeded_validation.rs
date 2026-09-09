// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::config::RuntimeConfig;

pub(super) fn seeded_tool_value(response: Value) -> Result<Value, String> {
    if response.get("error").is_some() {
        return Err(String::from("seeded-run MCP tool returned an RPC error"));
    }
    let content = response["result"]["content"]
        .as_array()
        .filter(|content| content.len() == 1)
        .ok_or_else(|| String::from("seeded-run MCP tool omitted bounded text content"))?;
    let text = content[0]["text"]
        .as_str()
        .ok_or_else(|| String::from("seeded-run MCP tool omitted text content"))?;
    serde_json::from_str(text)
        .map_err(|error| format!("seeded-run MCP tool returned invalid JSON: {error}"))
}

const SEEDED_RUN_ROOT_FIELDS: [&str; 20] = [
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
    "operation_id",
    "requested_seed",
    "run_mode",
    "selected_context",
    "context_digest",
    "status",
    "canonical_seed",
    "observation",
    "effect_witness",
    "error_code",
];

pub(super) fn validate_seeded_response(
    value: &Value,
    config: &RuntimeConfig,
    seed: &super::super::seed_transport::SeedTransportConfig,
    request_generation: u64,
    expected_kind: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| String::from("seeded-run response is not an object"))?;
    if object.len() != SEEDED_RUN_ROOT_FIELDS.len()
        || SEEDED_RUN_ROOT_FIELDS
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err(String::from(
            "seeded-run response has an invalid root shape",
        ));
    }
    if value["protocol_version"] != super::super::seed_transport::SEEDED_RUN_PROTOCOL_VERSION
        || value["schema_digest"] != super::super::seed_transport::SEEDED_RUN_SCHEMA_DIGEST
    {
        return Err(String::from(
            "seeded-run response has unsupported protocol metadata",
        ));
    }
    let provenance = value["provenance"]
        .as_object()
        .ok_or_else(|| String::from("seeded-run response omitted provenance"))?;
    if provenance.len() != 3
        || provenance["artifact"] != super::super::seed_transport::SEEDED_RUN_ARTIFACT
        || provenance["source"] != super::super::seed_transport::SEEDED_RUN_SCHEMA_SOURCE
        || provenance["generator"] != super::super::seed_transport::SEEDED_RUN_GENERATOR
    {
        return Err(String::from(
            "seeded-run response provenance is unsupported",
        ));
    }
    if value["kind"].as_str() != Some(expected_kind)
        || !seeded_identity(value["correlation_id"].as_str(), "correlation_id")?
    {
        return Err(String::from(
            "seeded-run response kind or correlation is invalid",
        ));
    }
    for (field, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("session_id", config.session_id.as_str()),
        ("lease_id", config.lease_id.as_str()),
        ("operation_id", seed.operation_id.as_str()),
        ("requested_seed", seed.requested_seed.as_str()),
        ("run_mode", seed.run_mode.as_str()),
        ("context_digest", seed.context_digest()),
    ] {
        if value[field].as_str() != Some(expected) {
            return Err(format!(
                "seeded-run response {field} does not match request"
            ));
        }
    }
    let lease_epoch = seeded_generation(&value["lease_epoch"], "lease_epoch")?;
    if lease_epoch != config.lease_epoch {
        return Err(String::from(
            "seeded-run response lease epoch does not match",
        ));
    }
    let generation = seeded_generation(&value["generation"], "generation")?;
    if generation != request_generation {
        return Err(String::from(
            "seeded-run response generation does not match its request fence",
        ));
    }
    if value["selected_context"] != seed.selected_context() {
        return Err(String::from(
            "seeded-run response selected context does not match request",
        ));
    }
    let status = value["status"]
        .as_str()
        .ok_or_else(|| String::from("seeded-run response status is invalid"))?;
    if !matches!(
        status,
        "accepted" | "settled" | "rejected" | "unknown" | "cancelled"
    ) {
        return Err(String::from("seeded-run response status is unsupported"));
    }
    match status {
        "settled" => {
            if value["canonical_seed"].as_str().is_none()
                || !value["observation"].is_object()
                || !value["effect_witness"].is_object()
                || !value["error_code"].is_null()
            {
                return Err(String::from(
                    "settled seeded-run response has an invalid result shape",
                ));
            }
        }
        "accepted" => {
            if !value["canonical_seed"].is_null()
                || !value["observation"].is_null()
                || !value["effect_witness"].is_null()
                || !value["error_code"].is_null()
            {
                return Err(String::from(
                    "accepted seeded-run response has an invalid result shape",
                ));
            }
        }
        "rejected" | "unknown" | "cancelled" => {
            if !value["canonical_seed"].is_null()
                || !value["observation"].is_null()
                || !value["effect_witness"].is_null()
                || !seeded_identity(value["error_code"].as_str(), "error_code")?
            {
                return Err(String::from(
                    "non-settled seeded-run response has an invalid result shape",
                ));
            }
        }
        _ => return Err(String::from("seeded-run response status is unsupported")),
    }
    Ok(())
}

pub(super) fn validate_seeded_settlement(
    value: &Value,
    config: &RuntimeConfig,
    seed: &super::super::seed_transport::SeedTransportConfig,
    request_generation: u64,
    expected_kind: &str,
) -> Result<(), String> {
    validate_seeded_response(value, config, seed, request_generation, expected_kind)?;
    if value["status"] != "settled" {
        return Err(String::from("seeded-run settlement did not settle"));
    }
    let canonical = value["canonical_seed"]
        .as_str()
        .ok_or_else(|| String::from("seeded-run settlement omitted canonical seed"))?;
    validate_seed(canonical, "canonical_seed")?;

    let observation = value["observation"]
        .as_object()
        .ok_or_else(|| String::from("seeded-run settlement omitted observation"))?;
    const OBSERVATION_FIELDS: [&str; 8] = [
        "run_started",
        "host_ready",
        "generation",
        "canonical_seed",
        "selected_context_digest",
        "phase_before",
        "phase_after",
        "compatibility_identity",
    ];
    if observation.len() != OBSERVATION_FIELDS.len()
        || OBSERVATION_FIELDS
            .iter()
            .any(|field| !observation.contains_key(*field))
    {
        return Err(String::from(
            "seeded-run settlement observation has an invalid shape",
        ));
    }
    if observation["run_started"] != true || observation["host_ready"] != true {
        return Err(String::from(
            "seeded-run settlement omitted a fresh host run-start witness",
        ));
    }
    let observation_generation =
        seeded_generation(&observation["generation"], "observation generation")?;
    if observation_generation <= request_generation {
        return Err(String::from(
            "seeded-run settlement observation did not advance generation",
        ));
    }
    if observation["canonical_seed"] != canonical
        || observation["selected_context_digest"] != seed.context_digest()
        || !seeded_identity(observation["phase_before"].as_str(), "phase_before")?
        || !seeded_identity(observation["phase_after"].as_str(), "phase_after")?
        || observation["phase_before"] == observation["phase_after"]
        || !seeded_identity(
            observation["compatibility_identity"].as_str(),
            "compatibility_identity",
        )?
    {
        return Err(String::from(
            "seeded-run settlement observation is not bound to the selected context",
        ));
    }
    let witness = value["effect_witness"]
        .as_object()
        .ok_or_else(|| String::from("seeded-run settlement omitted effect witness"))?;
    if witness.len() != 3
        || witness["kind"] != "run_started"
        || seeded_generation(&witness["generation"], "effect generation")? != observation_generation
        || witness["canonical_seed"] != canonical
    {
        return Err(String::from(
            "seeded-run settlement effect witness is not bound to observation",
        ));
    }
    Ok(())
}

fn seeded_generation(value: &Value, field: &str) -> Result<u64, String> {
    let generation = value
        .as_u64()
        .ok_or_else(|| format!("seeded-run {field} is not a non-negative integer"))?;
    if generation > super::super::seed_transport::SEEDED_RUN_MAX_GENERATION {
        return Err(format!("seeded-run {field} exceeds its safe integer bound"));
    }
    Ok(generation)
}

fn seeded_identity(value: Option<&str>, field: &str) -> Result<bool, String> {
    let Some(value) = value else {
        return Err(format!("seeded-run {field} is not a string"));
    };
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(format!("seeded-run {field} is unsafe or oversized"));
    }
    Ok(true)
}

fn validate_seed(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || value.bytes().any(|byte| byte <= 0x1f || byte == 0x7f)
    {
        return Err(format!("seeded-run {field} is unsafe or oversized"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_v3_seeded_validation_tests.rs"]
mod seeded_run_tests;
