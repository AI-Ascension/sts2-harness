// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, CommandKind, CommandParameters, CommandRequest, LiveWorkflowSession,
    LiveWorkflowSessionFactory, MANAGEMENT_SCHEMA_VERSION, RunRequest,
};
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DispatchStatus, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, TransitionReceipt, WaitOutcome,
    WaitSample,
};

pub(crate) const LIVE_CAPABILITIES: &[&str] = &[
    "workflow.live",
    "workflow.node.observe.v1",
    "workflow.node.decide.v1",
    "workflow.node.execute_action.v1",
    "workflow.node.terminal.v1",
    "workflow.projection.fair-play.live.v1",
    "workflow.provider.decision.live.v1",
    "workflow.context.context.live.v1",
    "observe.fair-play.live.v1",
    "actions.catalog.v1",
    "actions.settlement.v1",
];

pub(crate) fn actor() -> AuthContext {
    AuthContext::new("operator", ["workflow:*".to_owned()]).expect("actor")
}

pub(crate) fn definition(synthetic: bool) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    value["annotations"]["synthetic"] = json!(synthetic);
    if !synthetic {
        value["game_profile"] = json!("sts2-live-v1");
        value["policy_ref"] = json!("policy.live.v1");
        value["graphs"][0]["nodes"][0]["config"]["projection_ref"] = json!("fair-play.live.v1");
        value["graphs"][0]["nodes"][1]["config"]["decision_profile_ref"] =
            json!("decision.live.v1");
        value["graphs"][0]["nodes"][1]["config"]["context_ref"] = json!("context.live.v1");
        value["capabilities"]["required"][0] = json!("observe.fair-play.live.v1");
    }
    value
}

pub(crate) fn capabilities() -> Value {
    json!({
        "capabilities": LIVE_CAPABILITIES,
        "context_bindings": [{
            "context_ref": "context.live.v1",
            "node_kinds": ["decide"]
        }]
    })
}

pub(crate) fn request(id: &str, definition: Value) -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
    }
}

pub(crate) fn command(run_id: &str, id: &str, revision: u64, kind: CommandKind) -> CommandRequest {
    CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: id.to_owned(),
        run_id: run_id.to_owned(),
        expected_revision: revision,
        actor_scope: "operator".to_owned(),
        kind,
        parameters: CommandParameters::default(),
    }
}

pub(crate) fn observation(state_id: &str, generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "fixture",
            "player": {"hp": 50, "max_hp": 50, "energy": 3, "gold": 0,
                       "hand": [], "deck": [], "discard": [], "exhaust": []},
            "state": {"state": "combat", "turn_index": 1, "enemies": []},
            "legal_actions": [{"action_id": "end-turn", "action": {"kind": "end_turn"}}]
        }),
    )
    .expect("observation")
}

pub(crate) struct FakeFactory {
    log: Arc<Mutex<Vec<String>>>,
    unknown: bool,
    dispatch_error: bool,
}

impl FakeFactory {
    pub(crate) fn new(unknown: bool) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            unknown,
            dispatch_error: false,
        }
    }

    pub(crate) fn dispatch_error() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            unknown: false,
            dispatch_error: true,
        }
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.log.lock().expect("log").clone()
    }
}

impl LiveWorkflowSessionFactory for FakeFactory {
    fn capabilities(&self) -> Value {
        capabilities()
    }

    fn open(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &sts2_harness::workflow::WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, sts2_harness::management::ManagementError> {
        Ok(Box::new(FakeSession {
            log: Arc::clone(&self.log),
            unknown: self.unknown,
            dispatch_error: self.dispatch_error,
            identity: None,
            action: None,
        }))
    }
}

struct FakeSession {
    log: Arc<Mutex<Vec<String>>>,
    unknown: bool,
    dispatch_error: bool,
    identity: Option<String>,
    action: Option<EpisodeLegalAction>,
}

impl FakeSession {
    fn record(&self, value: &str) {
        self.log.lock().expect("log").push(value.to_owned());
    }
}

impl LiveWorkflowSession for FakeSession {
    fn launch(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        self.record("launch");
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::management::ManagementError> {
        self.record("observe");
        Ok(observation("state-0", 0))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::management::ManagementError> {
        self.record("legal_actions");
        EpisodeLegalActionSet::new(
            state_id,
            generation,
            vec![EpisodeLegalAction::new("end-turn", ActionKind::EndTurn).expect("action")],
        )
        .map_err(|error| {
            sts2_harness::management::ManagementError::invalid("fake_catalog", error.to_string())
        })
    }

    fn decide(
        &mut self,
        _input: &DecisionInput,
    ) -> Result<Decision, sts2_harness::management::ManagementError> {
        self.record("decide");
        Ok(Decision::Action {
            action_id: "end-turn".to_owned(),
            rationale: "fixture decision".to_owned(),
            confidence: Some(100),
        })
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::management::ManagementError> {
        self.record("dispatch");
        self.identity = Some(identity.operation_id.clone());
        self.action = Some(action.clone());
        if self.dispatch_error {
            return Err(sts2_harness::management::ManagementError::unresolved(
                "fake_transport",
                "dispatch response was lost",
            ));
        }
        let status = if self.unknown {
            DispatchStatus::Unknown
        } else {
            DispatchStatus::Accepted
        };
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            status,
            None,
            None,
            None,
        ))
    }

    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, sts2_harness::management::ManagementError> {
        self.record("wait");
        Ok(
            WaitSample::new(WaitOutcome::Successor, Some(observation("state-1", 1)))
                .with_effect_kind("host.semantic.test"),
        )
    }

    fn reconcile(
        &mut self,
        operation_id: &str,
    ) -> Result<TransitionReceipt, sts2_harness::management::ManagementError> {
        self.record("reconcile");
        let action = self.action.clone().ok_or_else(|| {
            sts2_harness::management::ManagementError::unavailable(
                "fake_reconcile",
                "missing operation",
            )
        })?;
        if self.identity.as_deref() != Some(operation_id) {
            return Err(sts2_harness::management::ManagementError::conflict(
                "fake_reconcile",
                "operation identity mismatch",
            ));
        }
        Ok(TransitionReceipt::new(
            operation_id,
            action,
            DispatchStatus::Settled,
            Some(observation("state-1", 1)),
            Some("host.semantic.reconciled".to_owned()),
            None,
        ))
    }

    fn release_lease(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), sts2_harness::management::ManagementError> {
        self.record("stop");
        Ok(())
    }
}
