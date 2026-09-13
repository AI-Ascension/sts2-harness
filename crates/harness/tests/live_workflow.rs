// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;
use sts2_harness::management::{
    CommandKind, EventType, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    WorkflowRunStatus, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

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
            "wait",
            "release"
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
            "reconcile",
            "release"
        ]
    );
}

#[test]
fn dispatch_error_keeps_pre_effect_intent_for_same_operation_reconciliation() {
    let factory = Arc::new(FakeFactory::dispatch_error());
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-transport", definition(false)))
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
    assert_eq!(
        service
            .status(&actor, &run_id)
            .expect("status")
            .run
            .pending_operation
            .as_ref()
            .expect("intent")
            .classification,
        sts2_harness::management::PendingOperationState::Intent
    );
    let events = service
        .events(&actor, &run_id, 0, 128)
        .expect("events")
        .events;
    assert!(events.iter().any(|event| {
        event.event_type == EventType::OperationIntent
            && event.payload.operation_id
                == service
                    .status(&actor, &run_id)
                    .expect("status")
                    .run
                    .pending_operation
                    .as_ref()
                    .map(|operation| operation.operation_id.clone())
    }));
    service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("reconcile");
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
