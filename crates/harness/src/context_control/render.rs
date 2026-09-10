// SPDX-License-Identifier: MIT

//! Pure provider preparation shared by preview and dispatch adapters.

use super::types::{
    ContextBoundary, ContextDraft, ContextItem, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS,
    MAX_CONTEXT_NOTES, MAX_NOTE_BYTES, MAX_OBJECTIVE_BYTES, ManagementProfile,
};
use crate::exo::{ExoConfig, ExoDecisionRequest, ExoError, SanitizedObservation};
use crate::identity::ModelExecutionId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const OUTPUT_SCHEMA: &[u8] = br#"{"type":"object","properties":{"action_id":{"type":"string"},"rationale":{"type":"string","maxLength":512}},"required":["action_id","rationale"],"additionalProperties":false}"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRenderInput {
    pub execution_id: String,
    pub state_id: String,
    pub generation: u64,
    pub observation: Value,
    pub legal_action_ids: Vec<String>,
    pub objective: String,
    pub hard_constraints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedContext {
    profile: ManagementProfile,
    provider_revision: String,
    max_response_bytes: usize,
    pub input: Vec<u8>,
    pub output_schema: Vec<u8>,
    pub configuration: Vec<u8>,
    pub manifest_sha256: String,
    model_input: Option<ManagedRenderInput>,
    management_context: Option<Value>,
}

impl PreparedContext {
    pub fn profile(&self) -> ManagementProfile {
        self.profile
    }

    pub fn provider_bytes(&self) -> &[u8] {
        &self.input
    }

    pub fn provider_revision(&self) -> &str {
        &self.provider_revision
    }

    pub fn reserved_execution_id(&self) -> Option<&str> {
        self.model_input
            .as_ref()
            .map(|fields| fields.execution_id.as_str())
    }

    /// Converts the frozen management input into the real Exo request type. The caller must use
    /// the reserved execution identity from the preview; changing it invalidates the bytes.
    pub fn exo_request(
        &self,
        execution_id: ModelExecutionId,
        config: &ExoConfig,
    ) -> Result<ExoDecisionRequest, ExoError> {
        if self.profile != ManagementProfile::Enabled {
            return Err(ExoError::InvalidRequest);
        }
        let fields = self.model_input.as_ref().ok_or(ExoError::InvalidRequest)?;
        if fields.execution_id != execution_id.to_string()
            || config.revision != self.provider_revision
        {
            return Err(ExoError::InvalidRequest);
        }
        let observation =
            SanitizedObservation::new(fields.observation.clone()).map_err(ExoError::Sandbox)?;
        let context = self
            .management_context
            .clone()
            .ok_or(ExoError::InvalidRequest)?;
        let value =
            serde_json::from_slice::<Value>(&self.input).map_err(|_| ExoError::InvalidRequest)?;
        let effective_objective = value
            .get("objective")
            .and_then(Value::as_str)
            .ok_or(ExoError::InvalidRequest)?;
        let request = ExoDecisionRequest::new_with_management(
            execution_id,
            self.provider_revision.clone(),
            fields.state_id.clone(),
            fields.generation,
            observation,
            fields.legal_action_ids.clone(),
            effective_objective,
            value
                .get("hard_constraints")
                .and_then(Value::as_array)
                .ok_or(ExoError::InvalidRequest)?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(ExoError::InvalidRequest)
                })
                .collect::<Result<Vec<_>, _>>()?,
            self.max_response_bytes,
            context,
        )?;
        let encoded = request.encode(config.max_request_bytes)?;
        let encoded_value =
            serde_json::from_slice::<Value>(&encoded).map_err(|_| ExoError::InvalidRequest)?;
        let prepared_value =
            serde_json::from_slice::<Value>(&self.input).map_err(|_| ExoError::InvalidRequest)?;
        if encoded_value != prepared_value {
            return Err(ExoError::InvalidRequest);
        }
        Ok(request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextRenderError {
    InvalidInput(&'static str),
    UnknownItem,
    ProtectedItem,
    ExpiredItem,
    InvalidUtf8,
    TooLarge,
    Encode,
}

impl std::fmt::Display for ContextRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput(message) => message,
            Self::UnknownItem => "selected item is unavailable",
            Self::ProtectedItem => "protected item cannot be selected for editing",
            Self::ExpiredItem => "selected item is expired",
            Self::InvalidUtf8 => "selected item is not valid UTF-8",
            Self::TooLarge => "prepared context exceeds its bound",
            Self::Encode => "prepared context encoding failed",
        })
    }
}

impl std::error::Error for ContextRenderError {}

#[derive(Clone, Debug, Default)]
pub struct ContextRenderer;

impl ContextRenderer {
    pub fn legacy(
        input: Vec<u8>,
        output_schema: Vec<u8>,
        configuration: Vec<u8>,
    ) -> Result<PreparedContext, ContextRenderError> {
        if input.is_empty()
            || input.len() > MAX_CONTEXT_BYTES
            || output_schema.is_empty()
            || output_schema.len() > MAX_CONTEXT_BYTES
            || configuration.len() > MAX_CONTEXT_BYTES
        {
            return Err(ContextRenderError::TooLarge);
        }
        let manifest_sha256 = manifest_digest(&input, &output_schema, &configuration, "legacy");
        Ok(PreparedContext {
            profile: ManagementProfile::Legacy,
            provider_revision: String::new(),
            max_response_bytes: 0,
            input,
            output_schema,
            configuration,
            manifest_sha256,
            model_input: None,
            management_context: None,
        })
    }

    pub fn enabled(
        boundary: &ContextBoundary,
        request: ManagedRenderInput,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        config: &ExoConfig,
    ) -> Result<PreparedContext, ContextRenderError> {
        Self::enabled_at(
            boundary,
            request,
            draft,
            registry,
            config,
            boundary.generation,
        )
    }

    /// Renders against an explicit logical clock. The compatibility `enabled` entry point uses
    /// the boundary generation as its deterministic clock; callers with a wall-clock or fixture
    /// clock should use this method so expiry is checked at the same instant as approval.
    pub fn enabled_at(
        boundary: &ContextBoundary,
        request: ManagedRenderInput,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        config: &ExoConfig,
        now: u64,
    ) -> Result<PreparedContext, ContextRenderError> {
        validate_request(&request)?;
        if draft.selected_items.len() > MAX_CONTEXT_ITEMS || draft.notes.len() > MAX_CONTEXT_NOTES {
            return Err(ContextRenderError::TooLarge);
        }
        if draft
            .selected_items
            .iter()
            .any(|reference| !reference.valid())
            || draft
                .notes
                .iter()
                .any(|note| !note.reference.valid() || !valid_attributed_to(&note.attributed_to))
            || draft
                .objective
                .as_ref()
                .is_some_and(|reference| !reference.valid())
        {
            return Err(ContextRenderError::InvalidInput(
                "context reference is invalid",
            ));
        }
        if draft.pinned_item_ids.len() > draft.selected_items.len()
            || draft.pinned_item_ids.iter().any(|item_id| {
                !draft
                    .selected_items
                    .iter()
                    .any(|reference| reference.item_id == *item_id)
            })
        {
            return Err(ContextRenderError::InvalidInput(
                "pinned item is not selected",
            ));
        }
        let mut selected = Vec::with_capacity(draft.selected_items.len());
        for reference in &draft.selected_items {
            let item = registry
                .get(&reference_key(reference))
                .ok_or(ContextRenderError::UnknownItem)?;
            if item.reference != *reference || digest(&item.bytes) != reference.sha256 {
                return Err(ContextRenderError::UnknownItem);
            }
            if item.protected {
                return Err(ContextRenderError::ProtectedItem);
            }
            if !item.editable(now) {
                return Err(ContextRenderError::ExpiredItem);
            }
            let content =
                std::str::from_utf8(&item.bytes).map_err(|_| ContextRenderError::InvalidUtf8)?;
            selected.push(json!({
                "item_id": item.reference.item_id,
                "version": item.reference.version,
                "sha256": item.reference.sha256,
                "kind": item.kind,
                "content": content,
            }));
        }
        let notes = draft
            .notes
            .iter()
            .map(|note| {
                let item = registry
                    .get(&reference_key(&note.reference))
                    .ok_or(ContextRenderError::UnknownItem)?;
                if item.reference != note.reference {
                    return Err(ContextRenderError::ExpiredItem);
                }
                if digest(&item.bytes) != note.reference.sha256 {
                    return Err(ContextRenderError::UnknownItem);
                }
                if !item.editable(now) {
                    return Err(ContextRenderError::ExpiredItem);
                }
                if item.bytes.len() > MAX_NOTE_BYTES {
                    return Err(ContextRenderError::TooLarge);
                }
                let content = std::str::from_utf8(&item.bytes)
                    .map_err(|_| ContextRenderError::InvalidUtf8)?;
                Ok(json!({
                    "item_id": item.reference.item_id,
                    "version": item.reference.version,
                    "sha256": item.reference.sha256,
                    "attributed_to": note.attributed_to,
                    "content": content,
                }))
            })
            .collect::<Result<Vec<_>, ContextRenderError>>()?;
        let objective = draft
            .objective
            .as_ref()
            .map(|reference| {
                let item = registry
                    .get(&reference_key(reference))
                    .ok_or(ContextRenderError::UnknownItem)?;
                if item.reference != *reference {
                    return Err(ContextRenderError::ExpiredItem);
                }
                if digest(&item.bytes) != reference.sha256 {
                    return Err(ContextRenderError::UnknownItem);
                }
                if !item.editable(now) {
                    return Err(ContextRenderError::ExpiredItem);
                }
                if item.bytes.len() > MAX_OBJECTIVE_BYTES {
                    return Err(ContextRenderError::TooLarge);
                }
                std::str::from_utf8(&item.bytes)
                    .map(str::to_owned)
                    .map_err(|_| ContextRenderError::InvalidUtf8)
            })
            .transpose()?;
        let managed_context = json!({
            "selected_items": selected,
            "pinned_item_ids": draft.pinned_item_ids,
            "notes": notes,
            "protected_boundary": {
                "state_id": boundary.state_id,
                "generation": boundary.generation,
                "observation_sha256": boundary.observation_sha256,
                "catalog_sha256": boundary.catalog_sha256,
                "authority": "host-owned"
            }
        });
        let input_value = json!({
            "schema": "sts2.exo-decision-v1",
            "provider_revision": config.revision,
            "model_execution_id": request.execution_id,
            "state_id": request.state_id,
            "generation": request.generation,
            "observation": request.observation,
            "legal_action_ids": request.legal_action_ids,
            "objective": objective.unwrap_or_else(|| request.objective.clone()),
            "hard_constraints": request.hard_constraints,
            "max_response_bytes": config.max_response_bytes,
            "management_profile": ManagementProfile::Enabled.as_str(),
            "management_context": managed_context
        });
        let input = serde_json::to_vec(&input_value).map_err(|_| ContextRenderError::Encode)?;
        let output_schema = OUTPUT_SCHEMA.to_vec();
        let configuration = serde_json::to_vec(&json!({
            "profile": ManagementProfile::Enabled.as_str(),
            "provider_revision": config.revision,
            "model_revision": boundary.model_revision,
            "configuration_sha256": boundary.configuration_sha256,
            "tools": [],
            "credentials": "excluded"
        }))
        .map_err(|_| ContextRenderError::Encode)?;
        if input.len() > MAX_CONTEXT_BYTES
            || output_schema.len() > MAX_CONTEXT_BYTES
            || configuration.len() > MAX_CONTEXT_BYTES
        {
            return Err(ContextRenderError::TooLarge);
        }
        let manifest_sha256 = manifest_digest(
            &input,
            &output_schema,
            &configuration,
            ManagementProfile::Enabled.as_str(),
        );
        Ok(PreparedContext {
            profile: ManagementProfile::Enabled,
            provider_revision: config.revision.clone(),
            max_response_bytes: config.max_response_bytes,
            input,
            output_schema,
            configuration,
            manifest_sha256,
            model_input: Some(request),
            management_context: Some(managed_context),
        })
    }
}

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

fn validate_request(request: &ManagedRenderInput) -> Result<(), ContextRenderError> {
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

fn reference_key(reference: &super::types::ContextItemRef) -> String {
    format!("{}:{}", reference.item_id, reference.version)
}

fn valid_attributed_to(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn manifest_digest(input: &[u8], schema: &[u8], configuration: &[u8], profile: &str) -> String {
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
