// SPDX-License-Identifier: MIT

use crate::game_information_validation::decode_strict;
use crate::sha256_hex;
use serde_json::{Value, json};
use std::sync::OnceLock;

use super::{LOOKUP_BINDING_SCHEMA_DIGEST, LookupBinding, LookupBindingError, LookupObservation};

impl std::fmt::Display for LookupBindingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "game-information lookup binding: {self:?}")
    }
}

impl std::error::Error for LookupBindingError {}

pub(super) struct Decoded {
    pub(super) kind: String,
    pub(super) state: String,
    pub(super) binding: LookupBinding,
    pub(super) observation: Option<LookupObservation>,
    pub(super) error: Option<LookupBindingError>,
}

pub(super) fn string(value: &Value, name: &str) -> Result<String, LookupBindingError> {
    value[name]
        .as_str()
        .map(str::to_owned)
        .ok_or(LookupBindingError::Invalid)
}

pub(super) fn binding_id(binding: &Value) -> Result<String, LookupBindingError> {
    let scope = &binding["scope"];
    let input = json!({
        "agent_id": string(scope, "agent_id")?,
        "authority_epoch": binding["authority_epoch"].as_u64().ok_or(LookupBindingError::Invalid)?,
        "content_manifest_id": string(binding, "content_manifest_id")?,
        "episode_id": string(scope, "episode_id")?,
        "game_profile": string(binding, "game_profile")?,
        "locale": string(binding, "locale")?,
        "project_id": string(scope, "project_id")?,
        "run_id": string(scope, "run_id")?,
    });
    serde_json::to_vec(&input)
        .map(sha256_hex)
        .map_err(|_| LookupBindingError::Invalid)
}

pub(super) fn schema(value: &Value) -> Result<(), LookupBindingError> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let source = include_str!(
                "../../../protocol-artifact/game-information-lookup-binding-v1/schema.json"
            );
            if sha256_hex(source) != LOOKUP_BINDING_SCHEMA_DIGEST {
                return Err(());
            }
            let schema = serde_json::from_str(source).map_err(|_| ())?;
            jsonschema::validator_for(&schema).map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| LookupBindingError::Invalid)?;
    if !validator.is_valid(value) {
        return Err(LookupBindingError::Invalid);
    }
    Ok(())
}

pub(super) fn error_code(value: &Value) -> LookupBindingError {
    match value["error"]["code"].as_str() {
        Some("unsupported_version") => LookupBindingError::UnsupportedVersion,
        Some("invalid_identity") => LookupBindingError::InvalidIdentity,
        Some("denied_scope") => LookupBindingError::DeniedScope,
        Some("missing_capability") => LookupBindingError::MissingCapability,
        Some("stale_snapshot") => LookupBindingError::StaleSnapshot,
        Some("mixed_binding") => LookupBindingError::MixedBinding,
        Some("reobserve_unavailable") => LookupBindingError::ReobserveUnavailable,
        _ => LookupBindingError::Invalid,
    }
}

/// Decode raw boundary bytes with the existing duplicate-member rejection.
pub fn decode_lookup_binding_response(bytes: &[u8]) -> Result<Value, LookupBindingError> {
    decode_strict(bytes).map_err(|_| LookupBindingError::Invalid)
}
