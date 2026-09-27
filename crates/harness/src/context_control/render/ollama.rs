// SPDX-License-Identifier: MIT

//! The shared Ollama projection and the digest helpers the renderer's pins are built from.
//!
//! `ollama_user_content` is compiled into the Ollama bridge as well as the harness: legacy
//! requests must return the exact old user content, and enabled requests add only the accepted,
//! bounded managed-context field. Keeping it here lets that projection be read against the
//! manifest digest it is checked with.

use super::super::types::{
    ContextItemRef, MAX_CONTEXT_BYTES, MAX_OBJECTIVE_BYTES, ManagementProfile,
};
use super::{ContextRenderError, ManagedRenderInput};
use crate::identity::ModelExecutionId;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Shared with the compiled Ollama bridge. Legacy requests return the exact old user content;
/// enabled requests add only the accepted, bounded managed context field.
pub fn ollama_user_content(request: &Value) -> Result<String, ContextRenderError> {
    let observation = request
        .get("observation")
        .ok_or(ContextRenderError::InvalidInput("observation is required"))?;
    let base = observation.to_string();
    if request.get("management_profile").and_then(Value::as_str)
        != Some(ManagementProfile::Enabled.as_str())
    {
        return Ok(base);
    }
    let context = request
        .get("management_context")
        .ok_or(ContextRenderError::InvalidInput(
            "management context is required",
        ))?;
    let context = serde_json::to_string(context).map_err(|_| ContextRenderError::Encode)?;
    if context.len() > MAX_CONTEXT_BYTES
        || base.len().saturating_add(context.len()) > MAX_CONTEXT_BYTES
    {
        return Err(ContextRenderError::TooLarge);
    }
    Ok(format!("{base}\n\n[management-context-v1]\n{context}"))
}

pub fn validate_request(request: &ManagedRenderInput) -> Result<(), ContextRenderError> {
    if request.execution_id.is_empty()
        || request.execution_id.len() > 128
        || request.state_id.is_empty()
        || request.state_id.len() > 128
        || request.legal_action_ids.is_empty()
        || request.legal_action_ids.len() > 256
        || request.hard_constraints.len() > 32
        || request.objective.is_empty()
        || request.objective.len() > MAX_OBJECTIVE_BYTES
    {
        return Err(ContextRenderError::InvalidInput(
            "managed request is outside its bound",
        ));
    }
    Ok(())
}

pub fn reference_key(reference: &ContextItemRef) -> String {
    format!("{}:{}", reference.item_id, reference.version)
}

pub fn parse_execution_id(value: &str) -> Result<ModelExecutionId, ContextRenderError> {
    value
        .strip_prefix("model-execution-")
        .and_then(|number| number.parse::<u64>().ok())
        .and_then(ModelExecutionId::new)
        .ok_or(ContextRenderError::InvalidInput(
            "managed execution identity is invalid",
        ))
}

pub fn valid_attributed_to(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn manifest_digest(input: &[u8], schema: &[u8], configuration: &[u8], profile: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ascension.context-control.manifest.v1\0");
    for (kind, bytes) in [
        ("input", input),
        ("output_schema", schema),
        ("configuration", configuration),
    ] {
        hasher.update(kind.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hasher.update(profile.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
