// SPDX-License-Identifier: MIT

//! Conversion of a prepared context into the provider's own request shape.
//!
//! `PreparedContext::exo_request` re-derives the request from the rendered bytes rather than
//! trusting the struct alongside them, then checks the two agree. That is deliberate: a rendered
//! input that does not match the request it claims to encode is refused here rather than sent to a
//! provider. Separated from the renderer so the conversion, and its consistency check, can be read
//! as one unit.

use super::super::types::ManagementProfile;
use super::{ManagedRenderInput, PreparedContext};
use crate::exo::{ExoConfig, ExoDecisionRequest, ExoError, SanitizedObservation};
use crate::identity::ModelExecutionId;
use serde_json::Value;

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

    pub fn matches_input(&self, input: &ManagedRenderInput) -> bool {
        self.model_input.as_ref() == Some(input)
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
        let map_context = match fields.map_context.as_ref() {
            Some(value) => Some(
                crate::episode::map::MapDecisionContext::from_exo_value(
                    value,
                    &fields.state_id,
                    fields.generation,
                    &fields.legal_action_ids,
                )
                .map_err(|_| ExoError::InvalidRequest)?,
            ),
            None => None,
        };
        let request = if let Some(map_context) = map_context {
            ExoDecisionRequest::new_with_map_and_management(
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
                map_context,
                context,
            )?
        } else {
            ExoDecisionRequest::new_with_management(
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
            )?
        };
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
