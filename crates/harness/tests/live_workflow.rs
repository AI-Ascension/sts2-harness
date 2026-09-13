// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, CommandKind, CommandParameters, CommandRequest, LiveWorkflowOptions,
    LiveWorkflowSession, LiveWorkflowSessionFactory, MANAGEMENT_SCHEMA_VERSION,
    MemoryWorkflowStore, RunRequest, WorkflowRunStatus, live_store,
};
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DispatchStatus, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, TransitionReceipt, WaitOutcome,
    WaitSample,
};

const LIVE_CAPABILITIES: &[&str] = &[
    "workflow.live",
    "workflow.node.observe.v1",
    "workflow.node.decide.v1",
    "workflow.node.execute_action.v1",
    "workflow.node.terminal.v1",
    "observe.fair-play.v1",
    "actions.catalog.v1",
    "actions.settlement.v1",
];

fn actor() -> AuthContext {
    AuthContext::new("operator", ["workflow:*".to_owned()]).expect("actor")
}

fn definition(synthetic: bool) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    value["annotations"]["synthetic"] = json!(synthetic);
    value
}

fn capabilities() -> Value {
    json!({ "capabilities": LIVE_CAPABILITIES })
}

fn request(id: &str, definition: Value) -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
    }
}

fn command(run_id: &str, id: &str, revision: u64, kind: CommandKind) -> CommandRequest {
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

fn observation(state_id: &str, generation: u64) -> EpisodeObservation {
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

struct FakeFactory {
    log: Arc<Mutex<Vec<String>>>,
    unknown: bool,
}

impl FakeFactory {
    fn new(unknown: bool) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            unknown,
        }
    }

    fn entries(&self) -> Vec<String> {
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
            identity: None,
            action: None,
        }))
    }
}

struct FakeSession {
    log: Arc<Mutex<Vec<String>>>,
    unknown: bool,
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

#[test]
fn authored_graph_calls_live_ports_in_order_and_settles_before_terminal() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-1", definition(false)))
        .expect("submit");
    assert_eq!(submitted.status, WorkflowRunStatus::Running);
    let run_id = submitted.workflow_run_id;

    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");
    service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("decide");
    service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("execute");
    let terminal = service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("terminal");
    assert_eq!(terminal.run_revision, 5);
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Completed
    );
    assert_eq!(
        factory.entries(),
        [
            "launch",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "wait"
        ]
    );
}

#[test]
fn unknown_receipt_is_persisted_and_reconciled_without_redispatch() {
    let factory = Arc::new(FakeFactory::new(true));
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-2", definition(false)))
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }
    let pending = service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("unknown");
    assert_eq!(
        pending.outcome,
        sts2_harness::management::CommandOutcome::Pending
    );
    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::NeedsOperator);
    assert!(snapshot.pending_operation.is_some());
    service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("reconcile");
    service
        .command(&actor, command(&run_id, "step-5", 5, CommandKind::Step))
        .expect("terminal");
    assert_eq!(
        factory.entries(),
        [
            "launch",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "reconcile"
        ]
    );
}

#[test]
fn unsupported_node_is_rejected_before_live_launch() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let mut value = definition(false);
    value["graphs"][0]["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({
            "id": "unsupported",
            "kind": "pause",
            "config": {"reason_code": "operator"}
        }));
    let error = service
        .submit_run(&actor(), request("request-live-3", value))
        .expect_err("unsupported node");
    assert_eq!(error.code, "definition_invalid");
    assert!(factory.entries().is_empty());
}

#[test]
fn cancellation_dominates_pause_and_stops_the_live_session() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-4", definition(false)))
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    service
        .command(&actor, command(&run_id, "pause", 1, CommandKind::Pause))
        .expect("pause");
    let cancelled = service
        .command(&actor, command(&run_id, "cancel", 2, CommandKind::Cancel))
        .expect("cancel");
    assert_eq!(cancelled.run_revision, 3);
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Cancelled
    );
    assert!(factory.entries().contains(&"stop".to_owned()));
}
