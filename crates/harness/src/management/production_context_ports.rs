// SPDX-License-Identifier: MIT

use super::*;
use crate::management::{
    ContextOwnerControlLimits, ContextRenderSource, ContextRenderSourceIdentity,
};

/// Receives the authoritative MCP observation and the run-reservation control
/// bound used by the served context owner. The owner composes its current
/// invocation binding before any delegated control effect.
pub trait LiveContextObservationPort: Send + Sync {
    fn record_observation(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        observation: &EpisodeObservation,
        control_limits: &ContextOwnerControlLimits,
    ) -> Result<(), ManagementError>;

    fn record_legal_actions(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError>;

    fn invalidate(&self, actor: &AuthContext, request: &RunRequest, definition_digest: &str);
}

/// Resolves the authoritative encrypted source used by one actual provider
/// invocation and rechecks its owner-issued fence before and after inference.
pub trait LiveContextRenderPort: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn render_source_for_decision(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError>;

    #[allow(clippy::too_many_arguments)]
    fn assert_render_source_current(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError>;

    fn render_required(&self) -> bool;
}
