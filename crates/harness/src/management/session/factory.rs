// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::auth::AuthContext;
use super::super::super::contract::{RunRequest, TargetCatalogResponse};
use super::super::super::service::ManagementError;
use super::{EpisodeRuntimeSession, LiveWorkflowSession};
use crate::episode::{DecisionSource, EpisodeRuntimePort};

/// A factory opens one session for each admitted workflow run.
pub trait LiveWorkflowSessionFactory: Send + Sync {
    fn capabilities(&self) -> Value;

    /// Returns the actor-scoped catalog used to mint and revalidate live
    /// admission bindings. Factories that cannot provide authoritative target
    /// metadata fail closed rather than allowing client-supplied descriptors.
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Err(ManagementError::unavailable(
            "target_catalog_unavailable",
            "live target discovery is not attached to this workflow owner",
        ))
    }

    /// Returns the actor-scoped inference-profile catalog, or `None` when the
    /// owner serves none. A served catalog is authoritative: admission and
    /// dispatch resolve every decision/planner reference in it and refuse
    /// unknown, mismatched, revoked or unsupported revisions before inference.
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<super::super::super::contract::InferenceProfileCatalog>, ManagementError>
    {
        Ok(None)
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &crate::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError>;

    /// Opens a session with control limits selected by the admitted run
    /// reservation. Implementations that receive selected limits must enforce
    /// them before creating or using a control authority.
    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &crate::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&super::super::super::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        if control_limits.is_some() {
            return Err(ManagementError::capability(
                "selected_context_control_limits_unsupported",
                "live session factory cannot enforce the admitted context control limits",
            ));
        }
        self.open(request, actor, definition, definition_digest)
    }
}

/// Closure-backed factory suitable for a process that owns concrete adapters.
pub struct LiveWorkflowFactory<F> {
    capabilities: Value,
    opener: F,
}

impl<F> LiveWorkflowFactory<F> {
    pub fn new(capabilities: Value, opener: F) -> Result<Self, ManagementError> {
        super::super::validation::validate_capability_manifest(&capabilities)?;
        Ok(Self {
            capabilities,
            opener,
        })
    }
}

impl<F, R, S> LiveWorkflowSessionFactory for LiveWorkflowFactory<F>
where
    F: Fn(
            &RunRequest,
            &AuthContext,
            &crate::workflow::WorkflowDefinition,
            &str,
        ) -> Result<(R, S), ManagementError>
        + Send
        + Sync,
    R: EpisodeRuntimePort + Send + 'static,
    S: DecisionSource + Send + 'static,
{
    fn capabilities(&self) -> Value {
        self.capabilities.clone()
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &crate::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        let (runtime, source) = (self.opener)(request, actor, definition, definition_digest)?;
        Ok(Box::new(EpisodeRuntimeSession::new(runtime, source)))
    }
}
