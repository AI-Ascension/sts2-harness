// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;
use sts2_harness::management::{
    CommandKind, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    WorkflowRunStatus, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

#[test]
fn restart_with_durable_intent_fails_closed_without_redispatch() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(FakeFactory::dispatch_error());
    let service = live_store(
        Arc::clone(&store) as Arc<dyn sts2_harness::management::WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-restart", definition(false)))
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }
    service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("unknown");
    drop(service);

    let restarted_factory = Arc::new(FakeFactory::new(false));
    let restarted = live_store(
        Arc::clone(&store) as Arc<dyn sts2_harness::management::WorkflowStore>,
        Arc::clone(&restarted_factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("restarted service");
    let error = restarted
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect_err("restart must fail closed");
    assert_eq!(error.code, "live_runtime_after_restart");
    assert!(restarted_factory.entries().is_empty());
    assert_eq!(
        restarted
            .status(&actor, &run_id)
            .expect("status")
            .run
            .pending_operation
            .expect("durable intent")
            .classification,
        sts2_harness::management::PendingOperationState::Intent
    );
}

#[test]
fn unavailable_authored_binding_is_rejected_before_live_launch() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let mut value = definition(false);
    value["graphs"][0]["nodes"][0]["config"]["projection_ref"] =
        json!("projection.not-advertised.v1");
    let error = service
        .submit_run(&actor(), request("request-live-binding", value))
        .expect_err("binding must be rejected");
    assert_eq!(error.code, "definition_invalid");
    assert!(factory.entries().is_empty());
}

#[test]
fn mismatched_settled_receipt_remains_unknown_with_original_identity() {
    let factory = Arc::new(FakeFactory::mismatched_receipt());
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-mismatch", definition(false)))
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
            .expect("intent")
            .classification,
        sts2_harness::management::PendingOperationState::Intent
    );
    assert_eq!(
        factory.entries().last().map(String::as_str),
        Some("dispatch")
    );
}

#[test]
fn cancel_attempts_cleanup_after_reconciliation_conflict() {
    let factory = Arc::new(FakeFactory::reconcile_conflict());
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-live-cancel-conflict", definition(false)),
        )
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }
    service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("unknown");
    let error = service
        .command(&actor, command(&run_id, "cancel", 4, CommandKind::Cancel))
        .expect_err("reconciliation conflict must be reported");
    assert_eq!(error.code, "live_reconcile_identity");
    assert!(factory.entries().contains(&"stop".to_owned()));
    assert!(factory.entries().contains(&"release".to_owned()));
}

#[test]
fn launch_failure_releases_partial_session() {
    let factory = Arc::new(FakeFactory::launch_error());
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let error = service
        .submit_run(
            &actor(),
            request("request-live-launch-failure", definition(false)),
        )
        .expect_err("launch failure");
    assert_eq!(error.code, "fake_launch");
    assert_eq!(factory.entries(), ["launch", "stop", "release"]);
}

#[test]
fn cleanup_failure_is_visible_after_cancel() {
    let factory = Arc::new(FakeFactory::cleanup_error());
    let service = live_store(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-live-cleanup", definition(false)))
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    service
        .command(&actor, command(&run_id, "cancel", 1, CommandKind::Cancel))
        .expect("cancel response");
    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(
        snapshot.cleanup,
        sts2_harness::management::CleanupState::NeedsOperator
    );
    assert!(factory.entries().contains(&"stop".to_owned()));
    assert!(factory.entries().contains(&"release".to_owned()));
}
