// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]
#![allow(dead_code)]
#![allow(unused_imports)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, CommandKind, CommandParameters, CommandRequest, LiveWorkflowSession,
    LiveWorkflowSessionFactory, MANAGEMENT_SCHEMA_VERSION, RunRequest, TargetCatalogResponse,
};
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DispatchStatus, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, TransitionReceipt, WaitOutcome,
    WaitSample,
};

#[path = "live_workflow_admission.rs"]
mod admission;

pub(crate) use admission::request;

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

#[derive(Clone)]
pub(crate) struct LaunchRecord {
    pub(crate) definition_digest: String,
    pub(crate) node_order: Vec<String>,
}

pub(crate) struct FakeFactory {
    log: Arc<Mutex<Vec<String>>>,
    launches: Arc<Mutex<Vec<LaunchRecord>>>,
    completions: Arc<Mutex<Vec<bool>>>,
    unknown: bool,
    dispatch_error: bool,
    decide_error: bool,
    mismatched_receipt: bool,
    reconcile_conflict: bool,
    launch_error: bool,
    stop_error: bool,
    release_error: bool,
    reconcile_unknown: bool,
    reconcile_status: Option<DispatchStatus>,
}

impl FakeFactory {
    pub(crate) fn new(unknown: bool) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            launches: Arc::new(Mutex::new(Vec::new())),
            completions: Arc::new(Mutex::new(Vec::new())),
            unknown,
            dispatch_error: false,
            decide_error: false,
            mismatched_receipt: false,
            reconcile_conflict: false,
            launch_error: false,
            stop_error: false,
            release_error: false,
            reconcile_unknown: false,
            reconcile_status: None,
        }
    }

    pub(crate) fn dispatch_error() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            launches: Arc::new(Mutex::new(Vec::new())),
            completions: Arc::new(Mutex::new(Vec::new())),
            unknown: false,
            dispatch_error: true,
            decide_error: false,
            mismatched_receipt: false,
            reconcile_conflict: false,
            launch_error: false,
            stop_error: false,
            release_error: false,
            reconcile_unknown: false,
            reconcile_status: None,
        }
    }

    /// A live session whose `decide` node fails before any provider call, exercising the
    /// generic runtime-fault arm rather than the accepted/unknown dispatch paths.
    pub(crate) fn decide_error() -> Self {
        Self::new(false).with_decide_error()
    }

    pub(crate) fn mismatched_receipt() -> Self {
        Self::new(false).with_mismatched_receipt()
    }

    pub(crate) fn reconcile_conflict() -> Self {
        Self::new(true).with_reconcile_conflict()
    }

    pub(crate) fn launch_error() -> Self {
        Self::new(false).with_launch_error()
    }

    pub(crate) fn cleanup_error() -> Self {
        Self::new(false).with_release_error()
    }

    pub(crate) fn unresolved_reconcile() -> Self {
        Self::new(true).with_reconcile_unknown()
    }

    pub(crate) fn reconciled(status: DispatchStatus) -> Self {
        Self::new(true).with_reconcile_status(status)
    }

    fn with_mismatched_receipt(mut self) -> Self {
        self.mismatched_receipt = true;
        self
    }

    fn with_reconcile_conflict(mut self) -> Self {
        self.reconcile_conflict = true;
        self
    }

    fn with_launch_error(mut self) -> Self {
        self.launch_error = true;
        self
    }

    fn with_decide_error(mut self) -> Self {
        self.decide_error = true;
        self
    }

    fn with_release_error(mut self) -> Self {
        self.release_error = true;
        self
    }

    fn with_reconcile_unknown(mut self) -> Self {
        self.reconcile_unknown = true;
        self
    }

    fn with_reconcile_status(mut self, status: DispatchStatus) -> Self {
        self.reconcile_status = Some(status);
        self
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.log.lock().expect("log").clone()
    }

    /// Records, for every opened live session, the validated definition digest and node order.
    pub(crate) fn launches(&self) -> Vec<LaunchRecord> {
        self.launches.lock().expect("launch log").clone()
    }

    pub(crate) fn completions(&self) -> Vec<bool> {
        self.completions.lock().expect("completion log").clone()
    }
}

impl LiveWorkflowSessionFactory for FakeFactory {
    fn capabilities(&self) -> Value {
        capabilities()
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, sts2_harness::management::ManagementError> {
        Ok(admission::target_catalog())
    }

    fn open(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, sts2_harness::management::ManagementError> {
        let nodes = definition
            .graphs
            .iter()
            .flat_map(|graph| graph.nodes.iter())
            .map(|node| node.id().to_string())
            .collect();
        self.launches
            .lock()
            .expect("launch log")
            .push(LaunchRecord {
                definition_digest: definition_digest.to_owned(),
                node_order: nodes,
            });
        Ok(Box::new(FakeSession {
            log: Arc::clone(&self.log),
            completions: Arc::clone(&self.completions),
            unknown: self.unknown,
            dispatch_error: self.dispatch_error,
            decide_error: self.decide_error,
            mismatched_receipt: self.mismatched_receipt,
            reconcile_conflict: self.reconcile_conflict,
            launch_error: self.launch_error,
            stop_error: self.stop_error,
            release_error: self.release_error,
            reconcile_unknown: self.reconcile_unknown,
            reconcile_status: self.reconcile_status,
            identity: None,
            action: None,
        }))
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, sts2_harness::management::ManagementError> {
        // This workflow test double has no delegated context-control operation; validate the
        // admitted bound before opening its non-control session. Production factories must pass
        // the same typed selection into the owner that creates the durable control authority.
        if control_limits.is_some_and(|limits| limits.max_control_events == 0) {
            return Err(sts2_harness::management::ManagementError::capability(
                "context_control_event_limit_invalid",
                "fake factory received an empty admitted control bound",
            ));
        }
        self.open(request, actor, definition, definition_digest)
    }
}

struct FakeSession {
    log: Arc<Mutex<Vec<String>>>,
    completions: Arc<Mutex<Vec<bool>>>,
    unknown: bool,
    dispatch_error: bool,
    decide_error: bool,
    mismatched_receipt: bool,
    reconcile_conflict: bool,
    launch_error: bool,
    stop_error: bool,
    release_error: bool,
    reconcile_unknown: bool,
    reconcile_status: Option<DispatchStatus>,
    identity: Option<String>,
    action: Option<EpisodeLegalAction>,
}

impl FakeSession {
    fn record(&self, value: &str) {
        self.log.lock().expect("log").push(value.to_owned());
    }
}

#[path = "live_workflow_context_owner.rs"]
mod context_owner;

pub(crate) use context_owner::{FakeContextOwner, live_service};

#[path = "live_workflow_session.rs"]
mod session;
