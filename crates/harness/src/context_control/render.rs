// SPDX-License-Identifier: MIT

//! Pure provider preparation shared by preview and dispatch adapters.

mod admission;
mod exo_request;
mod limits;
mod ollama;

pub use limits::{ContextRenderError, ContextRenderLimits};
pub use ollama::ollama_user_content;
pub use ollama::{
    digest, manifest_digest, parse_execution_id, reference_key, valid_attributed_to,
    validate_request,
};

use super::types::{
    ContextBoundary, ContextDraft, ContextItem, MAX_CONTEXT_BYTES, ManagementProfile,
};
use crate::exo::{ExoConfig, ExoDecisionRequest, SanitizedObservation};
use serde_json::{Value, json};
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
    pub map_context: Option<Value>,
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
        Self::enabled_at_with_limits(
            boundary,
            request,
            draft,
            registry,
            config,
            now,
            &ContextRenderLimits::harness_maxima(),
        )
    }

    /// Renders against the **selected** owner/profile limits. The harness maxima are checked first
    /// (unchanged), then the advertised limits are enforced with a precise error naming the limit,
    /// so a draft that the harness could prepare but the selected owner cannot accept is refused
    /// before any inference or retention rather than failing late.
    pub fn enabled_at_with_limits(
        boundary: &ContextBoundary,
        request: ManagedRenderInput,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        config: &ExoConfig,
        now: u64,
        limits: &ContextRenderLimits,
    ) -> Result<PreparedContext, ContextRenderError> {
        let admitted = admission::admit(&request, draft, registry, now, limits)?;
        let managed_context = json!({
            "selected_items": admitted.selected,
            "pinned_item_ids": draft.pinned_item_ids,
            "notes": admitted.notes,
            "protected_boundary": {
                "state_id": boundary.state_id,
                "generation": boundary.generation,
                "observation_sha256": boundary.observation_sha256,
                "catalog_sha256": boundary.catalog_sha256,
                "authority": "host-owned"
            }
        });
        let observation = SanitizedObservation::new(request.observation.clone())
            .map_err(|_| ContextRenderError::InvalidInput("managed observation is invalid"))?;
        let observation = if config.forward_visible_seed {
            observation
        } else {
            observation.without_visible_seed()
        };
        let provider_request = if let Some(map_value) = request.map_context.as_ref() {
            let map_context = crate::episode::map::MapDecisionContext::from_exo_value(
                map_value,
                &request.state_id,
                request.generation,
                &request.legal_action_ids,
            )
            .map_err(|_| ContextRenderError::InvalidInput("managed map context is invalid"))?;
            ExoDecisionRequest::new_with_map_and_management(
                parse_execution_id(&request.execution_id)?,
                config.revision.clone(),
                request.state_id.clone(),
                request.generation,
                observation,
                request.legal_action_ids.clone(),
                admitted.objective_text,
                request.hard_constraints.clone(),
                config.max_response_bytes,
                map_context,
                managed_context.clone(),
            )
            .map_err(|_| ContextRenderError::InvalidInput("managed provider request is invalid"))?
        } else {
            ExoDecisionRequest::new_with_management(
                parse_execution_id(&request.execution_id)?,
                config.revision.clone(),
                request.state_id.clone(),
                request.generation,
                observation,
                request.legal_action_ids.clone(),
                admitted.objective_text,
                request.hard_constraints.clone(),
                config.max_response_bytes,
                managed_context.clone(),
            )
            .map_err(|_| ContextRenderError::InvalidInput("managed provider request is invalid"))?
        };
        let input = provider_request
            .encode(config.max_request_bytes)
            .map_err(|_| ContextRenderError::TooLarge)?;
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
        if input.len() > limits.max_context_bytes {
            return Err(ContextRenderError::ExceedsSelectedLimit(
                "max_context_bytes",
            ));
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
