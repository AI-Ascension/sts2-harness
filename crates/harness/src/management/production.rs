// SPDX-License-Identifier: MIT

//! The served-live composition boundary.
//!
//! Gateway/MCP lifecycle and provider construction remain owned by their
//! respective adapters.  This module joins those already-authoritative ports
//! into one management session; it deliberately has no game transport.

use serde_json::Value;

use super::super::auth::AuthContext;
use super::super::contract::{RunRequest, TargetAdmissionBinding, TargetCatalogResponse};
use super::super::service::ManagementError;
use super::session::{LiveWorkflowSession, LiveWorkflowSessionFactory};
use crate::episode::{
    ActionIdentity, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeRuntimePort, RecoveryPort, TransitionReceipt, WaitSample,
};
use crate::workflow::WorkflowDefinition;

/// Authoritative, actor-scoped discovery supplied by the gateway/MCP owner.
pub trait LiveTargetCatalogPort: Send + Sync {
    fn target_catalog(&self, actor: &AuthContext)
    -> Result<TargetCatalogResponse, ManagementError>;
}

/// Opens the existing gateway/MCP runtime composition for one admitted run.
///
/// `revalidate_fence` must read the current gateway lease/generation and
/// refuse a stale admission before `open_runtime` can allocate, launch MCP,
/// or dispatch a game-facing operation.
pub trait LiveRuntimeSessionFactory: Send + Sync {
    fn revalidate_fence(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        admission: &TargetAdmissionBinding,
    ) -> Result<(), ManagementError>;

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
    catalog: Box<dyn LiveTargetCatalogPort>,
    runtime: Box<dyn LiveRuntimeSessionFactory>,
    provider: Box<dyn LiveProviderSessionFactory>,
}

impl ProductionLiveWorkflowSessionFactory {
    pub fn new(
        capabilities: Value,
        catalog: Box<dyn LiveTargetCatalogPort>,
        runtime: Box<dyn LiveRuntimeSessionFactory>,
        provider: Box<dyn LiveProviderSessionFactory>,
    ) -> Result<Self, ManagementError> {
        super::validation::validate_capability_manifest(&capabilities)?;
        Ok(Self {
            capabilities,
            catalog,
            runtime,
            provider,
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

    fn revalidate_fence(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        admission: &TargetAdmissionBinding,
    ) -> Result<(), ManagementError> {
        self.runtime
            .revalidate_fence(request, actor, definition, definition_digest, admission)
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
        let provider =
            self.provider
                .open_provider(request, actor, definition, definition_digest)?;
        Ok(Box::new(ProductionLiveWorkflowSession {
            runtime,
            provider,
        }))
    }
}

struct ProductionLiveWorkflowSession {
    runtime: Box<dyn EpisodeRuntimePort + Send>,
    provider: Box<dyn DecisionSource + Send>,
}

impl LiveWorkflowSession for ProductionLiveWorkflowSession {
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.runtime
            .launch()
            .map_err(runtime_error("live_launch_failed"))
    }

    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
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
        self.provider.decide(input).map_err(provider_error)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<crate::Decision, ManagementError> {
        self.provider
            .decide_for(input, decision_profile_ref, context_ref)
            .map_err(provider_error)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
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
        self.provider.action_completed(settled);
    }

    fn model_execution_id(&self) -> Option<crate::ModelExecutionId> {
        self.provider.model_execution_id()
    }
}

fn runtime_error(code: &'static str) -> impl FnOnce(crate::PortError) -> ManagementError {
    move |error| ManagementError::unavailable(code, error.to_string())
}

fn provider_error(error: crate::episode::PolicyError) -> ManagementError {
    ManagementError::unavailable("provider_decision_failed", error.to_string())
}
