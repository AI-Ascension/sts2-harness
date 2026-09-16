// SPDX-License-Identifier: MIT

//! The served-live composition boundary.
//!
//! Gateway/MCP lifecycle and provider construction remain owned by their
//! respective adapters.  This module joins those already-authoritative ports
//! into one management session; it deliberately has no game transport.

use serde_json::Value;
use std::sync::Arc;

use super::super::auth::AuthContext;
use super::super::contract::{RunRequest, TargetCatalogResponse};
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
        })
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

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
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
        }))
    }
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
}

impl LiveWorkflowSession for ProductionLiveWorkflowSession {
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.runtime
            .launch()
            .map_err(runtime_error("live_launch_failed"))?;
        let observation = self
            .runtime
            .observe()
            .map_err(runtime_error("live_launch_fence_failed"))?;
        self.launch_observation = Some(observation);
        self.provider_policy.load_active_policy(
            &self.actor,
            &self.request,
            &self.definition,
            &self.provider_capabilities,
        )?;
        self.provider = Some(self.provider_factory.open_provider(
            &self.request,
            &self.actor,
            &self.definition,
            &self.definition_digest,
        )?);
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
        if let Some(observation) = self.launch_observation.take() {
            return Ok(observation);
        }
        self.runtime
            .observe()
            .map_err(runtime_error("live_observe_failed"))
    }

    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        self.runtime
            .observe_projection(projection_ref)
            .map_err(runtime_error("live_observe_failed"))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError> {
        self.runtime
            .legal_actions(state_id, generation)
            .map_err(runtime_error("live_catalog_failed"))
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<crate::Decision, ManagementError> {
        self.assert_current_observation(&input.observation)?;
        self.provider_mut()?.decide(input).map_err(provider_error)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<crate::Decision, ManagementError> {
        self.assert_current_observation(&input.observation)?;
        self.provider_mut()?
            .decide_for(input, decision_profile_ref, context_ref)
            .map_err(provider_error)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
        let observation = self
            .runtime
            .observe()
            .map_err(runtime_error("live_action_fence_failed"))?;
        if observation.state_id() != identity.state_id
            || observation.generation() != identity.generation
        {
            return Err(ManagementError::conflict(
                "live_action_generation_stale",
                "the gateway/MCP observation changed before action dispatch; re-observe is required",
            ));
        }
        self.runtime
            .dispatch_action(identity, action)
            .map_err(runtime_error("live_dispatch_failed"))
    }

    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, ManagementError> {
        self.runtime
            .wait_for_transition(operation_id, wait_for_millis)
            .map_err(|error| {
                ManagementError::unresolved("live_transition_wait_failed", error.to_string())
            })
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, ManagementError> {
        self.runtime.reconcile(operation_id).map_err(|error| {
            ManagementError::unresolved("live_reconcile_failed", error.to_string())
        })
    }

    fn release_lease(&mut self) -> Result<(), ManagementError> {
        RecoveryPort::release_lease(&mut *self.runtime)
            .map_err(|error| ManagementError::unavailable("live_release_failed", error.to_string()))
    }

    fn stop_episode(&mut self) -> Result<(), ManagementError> {
        RecoveryPort::stop_episode(&mut *self.runtime)
            .map_err(|error| ManagementError::unavailable("live_stop_failed", error.to_string()))
    }

    fn action_completed(&mut self, settled: bool) {
        if let Some(provider) = self.provider.as_mut() {
            provider.action_completed(settled);
        }
    }

    fn model_execution_id(&self) -> Option<crate::ModelExecutionId> {
        self.provider
            .as_ref()
            .and_then(|provider| provider.model_execution_id())
    }
}

impl ProductionLiveWorkflowSession {
    /// The host cannot be atomically locked across a provider request. A
    /// fresh read therefore fences the provider call, and the runtime repeats
    /// the read at dispatch while the gateway/MCP action envelope carries its
    /// authoritative lease and generation fence.
    fn assert_current_observation(
        &mut self,
        expected: &EpisodeObservation,
    ) -> Result<(), ManagementError> {
        let current = self
            .runtime
            .observe()
            .map_err(runtime_error("live_provider_fence_failed"))?;
        if current.state_id() != expected.state_id()
            || current.generation() != expected.generation()
        {
            return Err(ManagementError::conflict(
                "live_provider_generation_stale",
                "the gateway/MCP observation changed before provider inference; re-observe is required",
            ));
        }
        Ok(())
    }

    fn provider_mut(
        &mut self,
    ) -> Result<&mut (dyn DecisionSource + Send + 'static), ManagementError> {
        self.provider.as_deref_mut().ok_or_else(|| {
            ManagementError::unavailable(
                "provider_session_not_open",
                "provider construction must follow the authoritative launch observation",
            )
        })
    }
}

fn runtime_error(code: &'static str) -> impl FnOnce(crate::PortError) -> ManagementError {
    move |error| ManagementError::unavailable(code, error.to_string())
}

fn provider_error(error: crate::episode::PolicyError) -> ManagementError {
    ManagementError::unavailable("provider_decision_failed", error.to_string())
}
