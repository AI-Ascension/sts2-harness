// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::episode::{
    ActionIdentity, BarrierError, BarrierPort, DecisionInput, DecisionSource, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, PolicyError, RecoveryError,
    RecoveryPort, ShutdownError, ShutdownPort, TransitionReceipt, WaitSample,
};
use crate::management::{
    AuthContext, LiveProviderPolicyPort, LiveProviderSessionFactory, ManagementError,
    ProviderSessionPolicyBinding, RunRequest, RuntimeAuthorityBinding, live_run_id,
};
use crate::provider_session::{NativeCapabilities, ProviderSessionPolicy, SessionScope};
use crate::workflow::WorkflowDefinition;
use crate::{Decision, PortError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ActivePolicy {
    sha256: String,
    journal_revision: u64,
    generation: u64,
}

#[derive(Clone, Copy)]
enum Change {
    AdoptDuringInference,
    HistoryOnlyDuringInference,
}

struct PolicyPort {
    active: Arc<Mutex<ActivePolicy>>,
    policy: ProviderSessionPolicy,
}

impl LiveProviderPolicyPort for PolicyPort {
    fn load_active_policy(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _workflow_run_id: &str,
        _definition: &WorkflowDefinition,
        _capabilities: &NativeCapabilities,
    ) -> Result<ProviderSessionPolicyBinding, ManagementError> {
        let active = self.active.lock().map_err(|_| {
            ManagementError::unavailable("test_policy_lock", "policy fixture lock was poisoned")
        })?;
        Ok(ProviderSessionPolicyBinding {
            policy: self.policy.clone(),
            policy_sha256: active.sha256.clone(),
            active_revision: active.journal_revision,
            adoption_generation: active.generation,
        })
    }
}

struct RecordingProvider {
    active: Arc<Mutex<ActivePolicy>>,
    change: Change,
    calls: Arc<AtomicUsize>,
}

impl RecordingProvider {
    fn decide_inner(&self) -> Result<Decision, PolicyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut active = self
            .active
            .lock()
            .map_err(|_| PolicyError::ProviderUnavailable)?;
        match self.change {
            Change::AdoptDuringInference => {
                active.sha256 = "b".repeat(64);
                active.generation += 1;
                active.journal_revision += 1;
            }
            Change::HistoryOnlyDuringInference => {
                active.journal_revision += 1;
            }
        }
        Ok(Decision::Wait {
            rationale: "recorded policy-fence test decision".to_owned(),
        })
    }
}

impl DecisionSource for RecordingProvider {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.decide_inner()
    }

    fn decide_for(
        &mut self,
        _input: &DecisionInput,
        _decision_profile_ref: &str,
        _context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        self.decide_inner()
    }
}

struct UnusedProviderFactory;

impl LiveProviderSessionFactory for UnusedProviderFactory {
    fn open_provider(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        Err(ManagementError::unavailable(
            "test_provider_unused",
            "provider factory is not used by direct session regression",
        ))
    }
}

struct Runtime(EpisodeObservation);

impl EpisodeRuntimePort for Runtime {
    fn launch(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        Ok(self.0.clone())
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
        Err(PortError::new(
            "test_dispatch_unused",
            "dispatch is not part of the policy-fence regression",
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
        Ok(self.0.clone())
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

fn make_session(
    change: Change,
) -> (
    ProductionLiveWorkflowSession,
    Arc<AtomicUsize>,
    Arc<Mutex<ActivePolicy>>,
) {
    let definition: WorkflowDefinition = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("workflow definition");
    let digest = "d".repeat(64);
    let request = RunRequest {
        schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "policy-fence-test".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: crate::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let run_id = live_run_id(&request, &digest).expect("run identity");
    let actor = AuthContext::new("test-actor", ["workflow:*".to_owned()]).expect("actor");
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
    let scope = SessionScope::new("test-project", "test-run", "test-episode", "test-agent")
        .expect("policy scope");
    let active = Arc::new(Mutex::new(ActivePolicy {
        sha256: "a".repeat(64),
        journal_revision: 4,
        generation: 0,
    }));
    let calls = Arc::new(AtomicUsize::new(0));
    let source = RecordingProvider {
        active: Arc::clone(&active),
        change,
        calls: Arc::clone(&calls),
    };
    let session = ProductionLiveWorkflowSession {
        runtime: Box::new(Runtime(observation)),
        provider: Some(Box::new(source)),
        provider_factory: Arc::new(UnusedProviderFactory),
        request,
        actor,
        definition,
        definition_digest: digest,
        launch_observation: None,
        provider_policy: Arc::new(PolicyPort {
            active: Arc::clone(&active),
            policy: ProviderSessionPolicy::disabled(scope),
        }),
        provider_capabilities: NativeCapabilities::fixture(),
        context_observations: None,
        context_render: None,
        context_control_limits: None,
        active_policy_binding: Some(("a".repeat(64), 0)),
        policy_change_fenced: false,
        authority_binding: RuntimeAuthorityBinding {
            instance_id: "test-instance".to_owned(),
            session_id: "test-session".to_owned(),
            lease_id: "test-lease".to_owned(),
            lease_epoch: 1,
            run_id,
            episode_id: "test-episode".to_owned(),
            trajectory_id: "test-trajectory".to_owned(),
            trace_id: "test-trace".to_owned(),
            artifact_id: "test-artifact".to_owned(),
            agent_id: "test-agent".to_owned(),
            adapter_revision: "test-adapter".to_owned(),
            model_revision: "test-model".to_owned(),
            configuration_digest: "b".repeat(64),
            output_schema_digest: "c".repeat(64),
        },
    };
    (session, calls, active)
}

/// Records a policy adoption landing while the run is idle: the active sha and
/// adoption generation both move, as the durable owner does on a real adopt.
fn adopt(active: &Arc<Mutex<ActivePolicy>>, sha256: &str, generation: u64) {
    let mut active = active.lock().expect("policy lock");
    active.sha256 = sha256.to_owned();
    active.generation = generation;
    active.journal_revision = active.journal_revision.saturating_add(1);
}

fn input() -> DecisionInput {
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
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![
            EpisodeLegalAction::new("combat.end-turn", crate::ActionKind::EndTurn)
                .expect("legal action"),
        ],
    )
    .expect("action set");
    DecisionInput::new(
        crate::ModelExecutionId::new(1).expect("execution"),
        observation,
        actions,
        "test objective",
        Vec::new(),
    )
}

#[test]
fn adoption_during_inference_discards_result_and_fences_next_decision() {
    let (mut session, calls, _) = make_session(Change::AdoptDuringInference);
    let error = session.decide(&input()).expect_err("changed adoption");
    assert_eq!(error.code, "provider_session_policy_changed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("session remains fenced");
    assert_eq!(error.code, "provider_session_policy_changed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// Adoption that lands while the run is idle (no in-flight `decide`) must be
/// picked up by the next decision instead of fencing the run for its lifetime.
/// Nothing was inferred under the superseded policy, so there is no stale result
/// to discard; issue #255.
#[test]
fn adopted_policy_before_idle_decide_is_picked_up_without_fencing() {
    let (mut session, calls, active) = make_session(Change::HistoryOnlyDuringInference);
    adopt(&active, &"b".repeat(64), 1);

    let decision = session.decide(&input()).expect("idle adoption is admitted");
    assert!(matches!(decision, Decision::Wait { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // The adopted identity is now the retained binding, so a second decision on
    // the unchanged active policy keeps passing.
    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("unchanged adopted policy stays admitted");
    assert!(matches!(decision, Decision::Wait { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A rebind at admission must not disable the in-flight discard fence: if the
/// policy changes again while the provider call is running, that result is still
/// discarded and the session stays fenced even after re-adopting the original
/// identity.
#[test]
fn in_flight_change_after_idle_rebind_still_fences_and_stays_fenced() {
    let (mut session, calls, active) = make_session(Change::AdoptDuringInference);
    adopt(&active, &"b".repeat(64), 1);

    let error = session.decide(&input()).expect_err("in-flight change");
    assert_eq!(error.code, "provider_session_policy_changed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    adopt(&active, &"b".repeat(64), 1);
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("fence is sticky");
    assert_eq!(error.code, "provider_session_policy_changed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn journal_only_change_with_same_active_generation_does_not_fence_decision() {
    let (mut session, calls, active) = make_session(Change::HistoryOnlyDuringInference);
    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("unchanged active policy");
    assert!(matches!(decision, Decision::Wait { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(active.lock().expect("policy lock").generation, 0);
    assert_eq!(active.lock().expect("policy lock").journal_revision, 5);
}

#[cfg(test)]
#[path = "production_managed_render_tests.rs"]
mod managed_render_tests;

#[cfg(test)]
#[path = "production_membership_render_tests.rs"]
mod membership_render_tests;
