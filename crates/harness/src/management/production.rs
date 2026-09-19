// SPDX-License-Identifier: MIT

//! The served-live composition boundary.
//!
//! Gateway/MCP lifecycle and provider construction remain owned by their
//! respective adapters.  This module joins those already-authoritative ports
//! into one management session; it deliberately has no game transport.

#[path = "production_context_ports.rs"]
mod context_ports;
pub use context_ports::{LiveContextObservationPort, LiveContextRenderPort};

use serde_json::Value;
use std::sync::Arc;

use super::super::auth::AuthContext;
use super::super::contract::{InferenceProfileCatalog, RunRequest, TargetCatalogResponse};
use super::super::inference_profile_binding::InferenceProfileBindingSet;
use super::super::inference_profile_catalog::LiveInferenceProfileCatalogPort;
use super::super::service::{LiveProviderPolicyPort, ManagementError};
use super::session::{LiveWorkflowSession, LiveWorkflowSessionFactory};
use crate::episode::{
    ActionIdentity, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeRuntimePort, RecoveryPort, TransitionReceipt, WaitSample,
};
use crate::provider_session::NativeCapabilities;
use crate::workflow::WorkflowDefinition;

/// Authoritative, actor-scoped discovery supplied by the gateway/MCP owner.
pub trait LiveTargetCatalogPort: Send + Sync {
    fn target_catalog(&self, actor: &AuthContext)
    -> Result<TargetCatalogResponse, ManagementError>;
}

/// Opens the existing gateway/MCP runtime composition for one admitted run.
///
/// The runtime allocates its lease in `launch`. Its first authoritative MCP
/// observation is retained by the served session as the generation fence
/// before the provider can be opened or an action can be dispatched.
pub trait LiveRuntimeSessionFactory: Send + Sync {
    fn open_runtime(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn EpisodeRuntimePort + Send>, ManagementError>;

    fn authority_binding(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<RuntimeAuthorityBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "runtime_authority_binding_unavailable",
            "runtime authority provenance is unavailable",
        ))
    }
}

/// Provenance captured from runtime configuration and admitted provider policy.
/// For an enforcing context owner, the configured lease pair is replaced with
/// the runtime's actual post-allocation lease before the first observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeAuthorityBinding {
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub trace_id: String,
    pub artifact_id: String,
    pub agent_id: String,
    pub adapter_revision: String,
    pub model_revision: String,
    pub configuration_digest: String,
    pub output_schema_digest: String,
}

/// Opens the provider decision source after the gateway fence has been
/// confirmed. Credentials remain confined to this owner-specific adapter.
pub trait LiveProviderSessionFactory: Send + Sync {
    fn open_provider(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError>;
}

/// Concrete served factory joining authoritative target discovery, the
/// existing gateway/MCP runtime, and the provider session.
pub struct ProductionLiveWorkflowSessionFactory {
    capabilities: Value,
    catalog: Arc<dyn LiveTargetCatalogPort>,
    runtime: Arc<dyn LiveRuntimeSessionFactory>,
    provider: Arc<dyn LiveProviderSessionFactory>,
    provider_policy: Arc<dyn LiveProviderPolicyPort>,
    provider_capabilities: NativeCapabilities,
    context_observations: Option<Arc<dyn LiveContextObservationPort>>,
    context_render: Option<Arc<dyn LiveContextRenderPort>>,
    inference_profiles: Option<Arc<dyn LiveInferenceProfileCatalogPort>>,
}

impl ProductionLiveWorkflowSessionFactory {
    pub fn new(
        capabilities: Value,
        catalog: Arc<dyn LiveTargetCatalogPort>,
        runtime: Arc<dyn LiveRuntimeSessionFactory>,
        provider: Arc<dyn LiveProviderSessionFactory>,
        provider_policy: Arc<dyn LiveProviderPolicyPort>,
        provider_capabilities: NativeCapabilities,
    ) -> Result<Self, ManagementError> {
        super::validation::validate_capability_manifest(&capabilities)?;
        provider_capabilities.validate().map_err(|error| {
            ManagementError::capability(
                "provider_session_capabilities_invalid",
                format!("provider session capability descriptor is invalid: {error}"),
            )
        })?;
        Ok(Self {
            capabilities,
            catalog,
            runtime,
            provider,
            provider_policy,
            provider_capabilities,
            context_observations: None,
            context_render: None,
            inference_profiles: None,
        })
    }

    /// Attaches the owner-served inference-profile catalog. Once attached,
    /// every decision reference is resolved in it before the runtime opens.
    pub fn with_inference_profile_catalog(
        mut self,
        catalog: Arc<dyn LiveInferenceProfileCatalogPort>,
    ) -> Self {
        self.inference_profiles = Some(catalog);
        self
    }

    pub fn with_context_observations(
        mut self,
        observations: Arc<dyn LiveContextObservationPort>,
    ) -> Self {
        self.context_observations = Some(observations);
        self
    }

    pub fn with_context_render_port(mut self, render: Arc<dyn LiveContextRenderPort>) -> Self {
        self.context_render = Some(render);
        self
    }
}

impl LiveWorkflowSessionFactory for ProductionLiveWorkflowSessionFactory {
    fn capabilities(&self) -> Value {
        self.capabilities.clone()
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.catalog.target_catalog(actor)
    }

    fn inference_profile_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<Option<InferenceProfileCatalog>, ManagementError> {
        self.inference_profiles
            .as_ref()
            .map(|port| port.inference_profile_catalog(actor))
            .transpose()
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.open_admitted(request, actor, definition, definition_digest, None)
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&super::super::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        if self.context_observations.is_some() && control_limits.is_none() {
            return Err(ManagementError::capability(
                "selected_context_control_limits_required",
                "served context observations require the admitted control limits",
            ));
        }
        if control_limits.is_some() && self.context_observations.is_none() {
            return Err(ManagementError::capability(
                "selected_context_control_owner_unavailable",
                "admitted context control limits have no attached enforcing owner",
            ));
        }
        if self
            .context_render
            .as_ref()
            .is_some_and(|render| render.render_required())
            && (self.context_observations.is_none() || control_limits.is_none())
        {
            return Err(ManagementError::capability(
                "selected_context_render_owner_unavailable",
                "managed rendering requires the admitted observation and control owner",
            ));
        }
        let admitted_profiles = session::inference_profile::admit_inference_profiles(
            self.inference_profiles.as_deref(),
            actor,
            request,
            definition,
        )?;
        let authority_binding =
            self.runtime
                .authority_binding(request, actor, definition, definition_digest)?;
        validate_runtime_authority_binding(request, definition_digest, &authority_binding)?;
        let runtime = self
            .runtime
            .open_runtime(request, actor, definition, definition_digest)?;
        Ok(Box::new(ProductionLiveWorkflowSession {
            runtime,
            provider: None,
            provider_factory: Arc::clone(&self.provider),
            request: request.clone(),
            actor: actor.clone(),
            definition: definition.clone(),
            definition_digest: definition_digest.to_owned(),
            launch_observation: None,
            provider_policy: Arc::clone(&self.provider_policy),
            provider_capabilities: self.provider_capabilities.clone(),
            context_observations: self.context_observations.clone(),
            context_render: self.context_render.clone(),
            context_control_limits: control_limits.cloned(),
            active_policy_binding: None,
            policy_change_fenced: false,
            authority_binding,
            inference_profiles: self.inference_profiles.clone(),
            admitted_profiles,
        }))
    }
}

fn validate_runtime_authority_binding(
    request: &RunRequest,
    definition_digest: &str,
    binding: &RuntimeAuthorityBinding,
) -> Result<(), ManagementError> {
    let workflow_run_id = super::execution_records::live_run_id(request, definition_digest)?;
    if binding.instance_id != request.instance_id
        || binding.run_id != workflow_run_id
        || binding.session_id.is_empty()
        || binding.lease_id.is_empty()
        || binding.lease_epoch == 0
        || binding.episode_id.is_empty()
        || binding.trajectory_id.is_empty()
        || binding.trace_id.is_empty()
        || binding.artifact_id.is_empty()
        || binding.agent_id.is_empty()
        || binding.adapter_revision.is_empty()
        || binding.model_revision.is_empty()
        || binding.configuration_digest.len() != 64
        || binding.output_schema_digest.len() != 64
    {
        return Err(ManagementError::conflict(
            "runtime_authority_scope_mismatch",
            "runtime authority is not bound to the admitted workflow run",
        ));
    }
    Ok(())
}

struct ProductionLiveWorkflowSession {
    runtime: Box<dyn EpisodeRuntimePort + Send>,
    provider: Option<Box<dyn DecisionSource + Send>>,
    provider_factory: Arc<dyn LiveProviderSessionFactory>,
    request: RunRequest,
    actor: AuthContext,
    definition: WorkflowDefinition,
    definition_digest: String,
    launch_observation: Option<EpisodeObservation>,
    provider_policy: Arc<dyn LiveProviderPolicyPort>,
    provider_capabilities: NativeCapabilities,
    context_observations: Option<Arc<dyn LiveContextObservationPort>>,
    context_render: Option<Arc<dyn LiveContextRenderPort>>,
    context_control_limits: Option<super::super::ContextOwnerControlLimits>,
    active_policy_binding: Option<(String, u64)>,
    policy_change_fenced: bool,
    authority_binding: RuntimeAuthorityBinding,
    inference_profiles: Option<Arc<dyn LiveInferenceProfileCatalogPort>>,
    /// Every inference binding resolved at admission; the run keeps these exact
    /// revisions and is never re-bound to a later catalog revision.
    admitted_profiles: Option<InferenceProfileBindingSet>,
}

#[path = "production/session.rs"]
mod session;

#[cfg(test)]
#[path = "production_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
