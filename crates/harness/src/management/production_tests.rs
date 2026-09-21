// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::episode::{
    ActionIdentity, BarrierError, BarrierPort, DecisionInput, DecisionSource, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, PolicyError, RecoveryError,
    RecoveryPort, RuntimeLeaseBinding, ShutdownError, ShutdownPort, TransitionReceipt, WaitSample,
};
use crate::management::{
    AuthContext, ContextOwnerControlLimits, LiveProviderPolicyPort, LiveRuntimeSessionFactory,
    LiveTargetCatalogPort, LiveWorkflowSessionFactory, ManagementError,
    ProviderSessionPolicyBinding, RunRequest, RuntimeAuthorityBinding, TargetCatalogResponse,
    live_run_id,
};
use crate::provider_session::{NativeCapabilities, ProviderSessionPolicy, SessionScope};
use crate::workflow::WorkflowDefinition;
use crate::{Decision, PortError};
use std::sync::{Arc, Mutex};

const STATIC_LEASE: &str = "configured-lease";
const ACQUIRED_LEASE: &str = "gateway-recovery-lease";
const SESSION_ID: &str = "runtime-session";

pub(super) type Shared<T> = Arc<Mutex<T>>;

/// `#94`: a repeated live run identity is refused before the reservation, the session open and the
/// episode launch, so a duplicate submission cannot buy a second effect.
#[cfg(test)]
#[path = "production_duplicate_run_tests.rs"]
mod duplicate_run_tests;

/// Counts the boundary crossings a fence is supposed to prevent.
#[derive(Default, PartialEq, Eq, Debug)]
pub(super) struct Counters {
    pub(super) runtime_opens: usize,
    pub(super) dispatch_calls: usize,
    pub(super) decide_calls: usize,
    pub(super) provider_opens: usize,
}

pub(super) struct Runtime {
    observation: EpisodeObservation,
    lease: RuntimeLeaseBinding,
    counters: Shared<Counters>,
}

impl EpisodeRuntimePort for Runtime {
    fn launch(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    fn current_lease_binding(&mut self) -> Result<RuntimeLeaseBinding, PortError> {
        Ok(self.lease.clone())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        Ok(self.observation.clone())
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        EpisodeLegalActionSet::new(
            state_id,
            generation,
            vec![
                EpisodeLegalAction::new("combat.end-turn", crate::ActionKind::EndTurn)
                    .map_err(|error| PortError::new("test_action", error.to_string(), false))?,
            ],
        )
        .map_err(|error| PortError::new("test_catalog", error.to_string(), false))
    }

    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        self.counters.lock().expect("counter lock").dispatch_calls += 1;
        Err(PortError::new(
            "test_dispatch_unused",
            "dispatch is not part of the lease-binding regression",
            false,
        ))
    }
}

impl BarrierPort for Runtime {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for Runtime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Ok(self.observation.clone())
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        Err(RecoveryError::Unsupported)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }
}

impl ShutdownPort for Runtime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
}

pub(super) struct RuntimeFactory {
    pub(super) authority: RuntimeAuthorityBinding,
    pub(super) acquired: RuntimeLeaseBinding,
    pub(super) observation: EpisodeObservation,
    pub(super) counters: Shared<Counters>,
}

impl LiveRuntimeSessionFactory for RuntimeFactory {
    fn open_runtime(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn EpisodeRuntimePort + Send>, ManagementError> {
        self.counters.lock().expect("counter lock").runtime_opens += 1;
        Ok(Box::new(Runtime {
            observation: self.observation.clone(),
            lease: self.acquired.clone(),
            counters: Arc::clone(&self.counters),
        }))
    }

    fn authority_binding(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<RuntimeAuthorityBinding, ManagementError> {
        Ok(self.authority.clone())
    }
}

pub(super) struct Catalog;

impl LiveTargetCatalogPort for Catalog {
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Err(ManagementError::unavailable(
            "test_catalog_unused",
            "catalog discovery is not exercised here",
        ))
    }
}

pub(super) struct Policy;

impl LiveProviderPolicyPort for Policy {
    fn load_active_policy(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _workflow_run_id: &str,
        _definition: &WorkflowDefinition,
        _capabilities: &NativeCapabilities,
    ) -> Result<ProviderSessionPolicyBinding, ManagementError> {
        Ok(ProviderSessionPolicyBinding {
            policy: ProviderSessionPolicy::disabled(
                SessionScope::new("test-project", "test-run", "test-episode", "test-agent")
                    .map_err(|error| {
                        ManagementError::invalid("test_policy_scope", error.to_string())
                    })?,
            ),
            policy_sha256: "a".repeat(64),
            active_revision: 1,
            adoption_generation: 0,
        })
    }
}

pub(super) struct Provider {
    pub(super) counters: Shared<Counters>,
}

impl LiveProviderSessionFactory for Provider {
    fn open_provider(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        self.counters.lock().expect("counter lock").provider_opens += 1;
        Ok(Box::new(NoopDecision {
            counters: Arc::clone(&self.counters),
        }))
    }
}

struct NoopDecision {
    counters: Shared<Counters>,
}

impl DecisionSource for NoopDecision {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.counters.lock().expect("counter lock").decide_calls += 1;
        Err(PolicyError::ProviderUnavailable)
    }
}

#[derive(Default)]
pub(super) struct CapturingOwner {
    pub(super) binding: Mutex<Option<RuntimeAuthorityBinding>>,
}

impl LiveContextObservationPort for CapturingOwner {
    fn record_observation(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        _observation: &EpisodeObservation,
        _control_limits: &ContextOwnerControlLimits,
    ) -> Result<(), ManagementError> {
        *self.binding.lock().map_err(|_| {
            ManagementError::unavailable("test_owner_lock", "capture lock was poisoned")
        })? = Some(binding.clone());
        Ok(())
    }

    fn record_legal_actions(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        _binding: &RuntimeAuthorityBinding,
        _actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError> {
        Ok(())
    }

    fn invalidate(&self, _actor: &AuthContext, _request: &RunRequest, _definition_digest: &str) {}
}

#[test]
fn production_session_hands_the_allocated_recovery_lease_to_the_context_owner() {
    let definition: WorkflowDefinition = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("workflow definition");
    let digest = "d".repeat(64);
    let request = RunRequest {
        schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "lease-handoff-test".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: crate::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let run_id = live_run_id(&request, &digest).expect("run identity");
    let authority = RuntimeAuthorityBinding {
        instance_id: request.instance_id.clone(),
        session_id: SESSION_ID.to_owned(),
        lease_id: STATIC_LEASE.to_owned(),
        lease_epoch: 1,
        run_id: run_id.clone(),
        episode_id: "test-episode".to_owned(),
        trajectory_id: "test-trajectory".to_owned(),
        trace_id: "test-trace".to_owned(),
        artifact_id: "test-artifact".to_owned(),
        agent_id: "test-agent".to_owned(),
        adapter_revision: "test-adapter".to_owned(),
        model_revision: "test-model".to_owned(),
        configuration_digest: "b".repeat(64),
        output_schema_digest: "c".repeat(64),
    };
    let acquired = RuntimeLeaseBinding {
        instance_id: request.instance_id.clone(),
        session_id: SESSION_ID.to_owned(),
        run_id,
        lease_id: ACQUIRED_LEASE.to_owned(),
        lease_epoch: 3,
    };
    let observation = EpisodeObservation::new(
        "combat-1",
        1,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":1,
            "visible_seed":"fixture",
            "player":{"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation");
    let owner = Arc::new(CapturingOwner::default());
    let counters = Arc::new(Mutex::new(Counters::default()));
    let factory = ProductionLiveWorkflowSessionFactory::new(
        serde_json::json!({"capabilities":[]}),
        Arc::new(Catalog),
        Arc::new(RuntimeFactory {
            authority,
            acquired,
            observation,
            counters: Arc::clone(&counters),
        }),
        Arc::new(Provider {
            counters: Arc::clone(&counters),
        }),
        Arc::new(Policy),
        NativeCapabilities::fixture(),
    )
    .expect("production factory")
    .with_context_observations(owner.clone());
    let actor = AuthContext::new("test-actor", ["workflow:*".to_owned()]).expect("actor");
    let limits = ContextOwnerControlLimits {
        schema_version: crate::management::CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA.to_owned(),
        owner_id: "test-owner".to_owned(),
        owner_version: "v1".to_owned(),
        catalog_digest: "e".repeat(64),
        max_control_events: 1,
    };
    let mut session = factory
        .open_admitted(&request, &actor, &definition, &digest, Some(&limits))
        .expect("admitted session");
    session.launch().expect("session launch");

    let observed = owner
        .binding
        .lock()
        .expect("capture lock")
        .clone()
        .expect("owner observation");
    assert_eq!(observed.lease_id, ACQUIRED_LEASE);
    assert_eq!(observed.lease_epoch, 3);
    assert_eq!(observed.session_id, SESSION_ID);
    assert_eq!(observed.instance_id, request.instance_id);
}
