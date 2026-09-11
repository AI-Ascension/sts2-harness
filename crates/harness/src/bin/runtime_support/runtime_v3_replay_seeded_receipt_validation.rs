// SPDX-License-Identifier: MIT

use serde_json::Value;

use crate::runtime_support::seed_transport::{
    SEEDED_RUN_ARTIFACT, SEEDED_RUN_GENERATOR, SEEDED_RUN_MAX_GENERATION,
    SEEDED_RUN_PROTOCOL_VERSION, SEEDED_RUN_SCHEMA_DIGEST, SEEDED_RUN_SCHEMA_SOURCE,
};

pub(super) fn shape(v: &Value, kind: &str) -> Result<(), String> {
    const FIELDS: [&str; 20] = [
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
    if v.as_object().is_none_or(|o| o.len() != FIELDS.len())
        || FIELDS.iter().any(|field| v.get(*field).is_none())
        || v["protocol_version"] != SEEDED_RUN_PROTOCOL_VERSION
        || v["schema_digest"] != SEEDED_RUN_SCHEMA_DIGEST
        || v["kind"] != kind
        || v["provenance"]["artifact"] != SEEDED_RUN_ARTIFACT
        || v["provenance"]["source"] != SEEDED_RUN_SCHEMA_SOURCE
        || v["provenance"]["generator"] != SEEDED_RUN_GENERATOR
    {
        return Err("seeded receipt response has unsupported provenance".into());
    }
    for field in [
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "operation_id",
        "run_mode",
    ] {
        id(&v[field], field)?;
    }
    seed(&v["requested_seed"], "requested_seed")?;
    digest(&v["context_digest"], "context_digest")?;
    num(&v["lease_epoch"], "lease_epoch")?;
    num(&v["generation"], "generation")?;
    if !v["selected_context"].is_object() {
        return Err("seeded receipt selected context is invalid".into());
    }
    match v["status"].as_str() {
        Some("accepted")
            if v["canonical_seed"].is_null()
                && v["observation"].is_null()
                && v["effect_witness"].is_null()
                && v["error_code"].is_null() =>
        {
            Ok(())
        }
        Some("unknown")
            if v["canonical_seed"].is_null()
                && v["observation"].is_null()
                && v["effect_witness"].is_null() =>
        {
            id(&v["error_code"], "error_code").map(|_| ())
        }
        Some("settled")
            if v["canonical_seed"].is_string()
                && v["observation"].is_object()
                && v["effect_witness"].is_object()
                && v["error_code"].is_null() =>
        {
            Ok(())
        }
        _ => Err("seeded receipt response has an invalid status result".into()),
    }
}

pub(super) fn id(v: &Value, field: &str) -> Result<String, String> {
    let text = v
        .as_str()
        .ok_or_else(|| format!("seeded receipt {field} is not a string"))?;
    if text.is_empty()
        || text.len() > 128
        || text.chars().any(char::is_control)
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(format!("seeded receipt {field} is unsafe or oversized"));
    }
    Ok(text.into())
}

pub(super) fn seed(v: &Value, field: &str) -> Result<String, String> {
    let text = v
        .as_str()
        .ok_or_else(|| format!("seeded receipt {field} is not a string"))?;
    if text.is_empty() || text.len() > 64 || text.bytes().any(|byte| byte <= 31 || byte == 127) {
        return Err(format!("seeded receipt {field} is unsafe or oversized"));
    }
    Ok(text.into())
}

pub(super) fn digest(v: &Value, field: &str) -> Result<String, String> {
    let text = v
        .as_str()
        .ok_or_else(|| format!("seeded receipt {field} is not a string"))?;
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!(
            "seeded receipt {field} is not a lowercase SHA-256 digest"
        ));
    }
    Ok(text.into())
}

pub(super) fn num(v: &Value, field: &str) -> Result<u64, String> {
    let number = v
        .as_u64()
        .ok_or_else(|| format!("seeded receipt {field} is invalid"))?;
    if number > SEEDED_RUN_MAX_GENERATION {
        return Err(format!("seeded receipt {field} exceeds its safe bound"));
    }
    Ok(number)
}
