// SPDX-License-Identifier: MIT

//! Served-live authority, lease and generation fences.
//!
//! `#94` requires that a wrong instance, a stale lease, an unavailable provider and an invalid
//! binding each produce *no* effect and no synthetic success, and that an observation that moved
//! on between the decision and the dispatch is refused rather than committed. The composition
//! under test is the real production session assembled by
//! [`ProductionLiveWorkflowSessionFactory`]; only the gateway/MCP runtime and the provider are
//! fixtures. Each negative case asserts the refusal code *and* that the guarded port was never
//! reached, so a fence that has silently stopped working cannot pass by refusal alone.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::tests::{CapturingOwner, Catalog, Counters, Policy, Provider, RuntimeFactory, Shared};
use super::*;
use crate::ModelExecutionId;
use crate::episode::{
    ActionIdentity, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeStage, RuntimeLeaseBinding,
};
use crate::management::{
    ContextOwnerControlLimits, LiveWorkflowSession, ManagementError, live_run_id,
};
use crate::workflow::WorkflowDefinition;
use std::sync::{Arc, Mutex};

/// One field-level mutation of an authority binding, named for the assertion message.
type AuthorityMutation = (&'static str, fn(&mut RuntimeAuthorityBinding));
/// One field-level mutation of a lease binding, named for the assertion message.
type LeaseMutation = (&'static str, fn(&mut RuntimeLeaseBinding));

/// A provider factory that fails the way an unavailable provider session does.
struct UnavailableProvider {
    counters: Shared<Counters>,
}

impl LiveProviderSessionFactory for UnavailableProvider {
    fn open_provider(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        self.counters.lock().unwrap().provider_opens += 1;
        Err(ManagementError::unavailable(
            "test_provider_unavailable",
            "the provider session could not be opened",
        ))
    }
}

fn observation(state_id: &str, generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "fixture",
            "player": {"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state": {"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions": [{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation")
}

/// One served-live composition whose runtime and provider are the shared fixtures.
struct Served {
    request: RunRequest,
    definition: WorkflowDefinition,
    digest: String,
    actor: AuthContext,
    limits: ContextOwnerControlLimits,
    counters: Shared<Counters>,
    owner: Arc<CapturingOwner>,
}

impl Served {
    fn new() -> Self {
        Self {
            request: RunRequest {
                schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
                request_id: "production-fence-test".to_owned(),
                definition: None,
                artifact_id: None,
                instance_id: "test-instance".to_owned(),
                profile: crate::management::LIVE_WORKFLOW_PROFILE.to_owned(),
                admission: None,
            },
            definition: serde_json::from_str(include_str!(
                "../../../../conformance/workflow-v1/valid-strict.json"
            ))
            .expect("workflow definition"),
            digest: "d".repeat(64),
            actor: AuthContext::new("test-actor", ["workflow:*".to_owned()]).expect("actor"),
            limits: ContextOwnerControlLimits {
                schema_version: crate::management::CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA.to_owned(),
                owner_id: "test-owner".to_owned(),
                owner_version: "v1".to_owned(),
                catalog_digest: "e".repeat(64),
                max_control_events: 1,
            },
            counters: Arc::new(Mutex::new(Counters::default())),
            owner: Arc::new(CapturingOwner::default()),
        }
    }

    fn run_id(&self) -> String {
        live_run_id(&self.request, &self.digest).expect("run identity")
    }

    fn authority(&self) -> RuntimeAuthorityBinding {
        RuntimeAuthorityBinding {
            instance_id: self.request.instance_id.clone(),
            session_id: "runtime-session".to_owned(),
            lease_id: "configured-lease".to_owned(),
            lease_epoch: 1,
            run_id: self.run_id(),
            episode_id: "test-episode".to_owned(),
            trajectory_id: "test-trajectory".to_owned(),
            trace_id: "test-trace".to_owned(),
            artifact_id: "test-artifact".to_owned(),
            agent_id: "test-agent".to_owned(),
            adapter_revision: "test-adapter".to_owned(),
            model_revision: "test-model".to_owned(),
            configuration_digest: "b".repeat(64),
            output_schema_digest: "c".repeat(64),
        }
    }

    fn acquired(&self) -> RuntimeLeaseBinding {
        RuntimeLeaseBinding {
            instance_id: self.request.instance_id.clone(),
            session_id: "runtime-session".to_owned(),
            run_id: self.run_id(),
            lease_id: "gateway-recovery-lease".to_owned(),
            lease_epoch: 3,
        }
    }

    fn factory(
        &self,
        authority: RuntimeAuthorityBinding,
        acquired: RuntimeLeaseBinding,
        observation: EpisodeObservation,
        provider_unavailable: bool,
    ) -> ProductionLiveWorkflowSessionFactory {
        let provider: Arc<dyn LiveProviderSessionFactory> = if provider_unavailable {
            Arc::new(UnavailableProvider {
                counters: Arc::clone(&self.counters),
            })
        } else {
            Arc::new(Provider {
                counters: Arc::clone(&self.counters),
            })
        };
        ProductionLiveWorkflowSessionFactory::new(
            serde_json::json!({"capabilities":[]}),
            Arc::new(Catalog),
            Arc::new(RuntimeFactory {
                authority,
                acquired,
                observation,
                counters: Arc::clone(&self.counters),
            }),
            provider,
            Arc::new(Policy),
            NativeCapabilities::fixture(),
        )
        .expect("production factory")
        .with_context_observations(self.owner.clone() as Arc<dyn LiveContextObservationPort>)
    }

    fn open(
        &self,
        factory: &ProductionLiveWorkflowSessionFactory,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        factory.open_admitted(
            &self.request,
            &self.actor,
            &self.definition,
            &self.digest,
            Some(&self.limits),
        )
    }

    fn counts(&self) -> (usize, usize, usize, usize) {
        let counters = self.counters.lock().unwrap();
        (
            counters.runtime_opens,
            counters.dispatch_calls,
            counters.decide_calls,
            counters.provider_opens,
        )
    }

    fn action(&self) -> EpisodeLegalAction {
        EpisodeLegalAction::new("combat.end-turn", crate::ActionKind::EndTurn).expect("action")
    }

    fn decision_input(&self, observation: EpisodeObservation) -> DecisionInput {
        let actions = EpisodeLegalActionSet::new(
            observation.state_id(),
            observation.generation(),
            vec![self.action()],
        )
        .expect("legal actions");
        DecisionInput::new(
            ModelExecutionId::new(1).expect("execution identity"),
            observation,
            actions,
            "execute the authored workflow",
            Vec::new(),
        )
    }
}

#[test]
fn served_open_refuses_an_authority_scope_outside_the_admitted_run() {
    // Each entry changes exactly one field of an otherwise valid binding.
    let cases: [AuthorityMutation; 7] = [
        ("empty lease id", |binding| binding.lease_id.clear()),
        ("zero lease epoch", |binding| binding.lease_epoch = 0),
        ("empty session id", |binding| binding.session_id.clear()),
        ("foreign run id", |binding| {
            binding.run_id = "run.live.foreign".to_owned();
        }),
        ("foreign instance", |binding| {
            binding.instance_id = "other-instance".to_owned();
        }),
        ("short configuration digest", |binding| {
            binding.configuration_digest = "short".to_owned();
        }),
        ("empty adapter revision", |binding| {
            binding.adapter_revision.clear();
        }),
    ];
    for (field, mutate) in cases {
        let served = Served::new();
        let mut authority = served.authority();
        mutate(&mut authority);
        let factory = served.factory(
            authority,
            served.acquired(),
            observation("combat-1", 1),
            false,
        );
        let error = served.open(&factory).map(drop).expect_err(field);
        assert_eq!(error.code, "runtime_authority_scope_mismatch", "{field}");
        assert_eq!(
            served.counts(),
            (0, 0, 0, 0),
            "{field}: an out-of-scope authority must not open the runtime"
        );
    }
}

#[test]
fn served_launch_refuses_a_post_allocation_lease_outside_the_admitted_scope() {
    let cases: [LeaseMutation; 5] = [
        ("foreign session", |lease| {
            lease.session_id = "other-session".to_owned();
        }),
        ("zero lease epoch", |lease| lease.lease_epoch = 0),
        ("empty lease id", |lease| lease.lease_id.clear()),
        ("foreign run", |lease| {
            lease.run_id = "run.live.foreign".to_owned();
        }),
        ("foreign instance", |lease| {
            lease.instance_id = "other-instance".to_owned();
        }),
    ];
    for (field, mutate) in cases {
        let served = Served::new();
        let mut acquired = served.acquired();
        mutate(&mut acquired);
        let factory = served.factory(
            served.authority(),
            acquired,
            observation("combat-1", 1),
            false,
        );
        let mut session = served.open(&factory).expect("admitted session");
        let error = session.launch().expect_err(field);
        assert_eq!(error.code, "runtime_lease_binding_mismatch", "{field}");
        let (_, _, decide_calls, provider_opens) = served.counts();
        assert_eq!(
            (decide_calls, provider_opens),
            (0, 0),
            "{field}: a stale lease must not open a paid provider session"
        );
    }
}

#[test]
fn served_dispatch_refuses_a_stale_observation_before_the_gateway_sees_the_action() {
    let served = Served::new();
    // The identity was produced from generation 1; the gateway has already advanced to generation 2.
    let factory = served.factory(
        served.authority(),
        served.acquired(),
        observation("combat-1", 2),
        false,
    );
    let mut session = served.open(&factory).expect("admitted session");
    session.launch().expect("session launch");

    let identity = ActionIdentity::new("operation-1", "combat-1", 1, "combat.end-turn")
        .expect("action identity");
    let error = session
        .dispatch_action(&identity, &served.action())
        .expect_err("stale dispatch");
    assert_eq!(error.code, "live_action_generation_stale");
    let (_, dispatch_calls, ..) = served.counts();
    assert_eq!(
        dispatch_calls, 0,
        "the fenced dispatch must never reach the gateway/MCP runtime"
    );
}

#[test]
fn served_decide_refuses_a_stale_observation_without_reaching_the_provider() {
    let served = Served::new();
    let factory = served.factory(
        served.authority(),
        served.acquired(),
        observation("combat-1", 2),
        false,
    );
    let mut session = served.open(&factory).expect("admitted session");
    session.launch().expect("session launch");

    let input = served.decision_input(observation("combat-1", 1));
    let error = session.decide(&input).expect_err("stale inference");
    assert_eq!(error.code, "live_provider_generation_stale");
    let (_, _, decide_calls, _) = served.counts();
    assert_eq!(
        decide_calls, 0,
        "a stale observation must not be paid for at the provider"
    );
}

#[test]
fn served_launch_reports_an_unavailable_provider_without_a_paid_decision() {
    let served = Served::new();
    let factory = served.factory(
        served.authority(),
        served.acquired(),
        observation("combat-1", 1),
        true,
    );
    let mut session = served.open(&factory).expect("admitted session");
    let error = session.launch().expect_err("unavailable provider");
    assert_eq!(error.code, "test_provider_unavailable");
    let (runtime_opens, dispatch_calls, decide_calls, provider_opens) = served.counts();
    assert_eq!((runtime_opens, provider_opens), (1, 1));
    assert_eq!(
        (dispatch_calls, decide_calls),
        (0, 0),
        "an unavailable provider must not yield a synthetic success"
    );
}
