// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;
use sts2_harness::management::{
    CommandKind, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    WorkflowRunStatus, WorkflowStore, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

#[test]
fn restart_with_durable_intent_fails_closed_without_redispatch() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(FakeFactory::dispatch_error());
    let service = live_service(
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
    let restarted = live_service(
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
fn duplicate_live_identity_is_refused_before_any_live_effect() {
    let seed_store = Arc::new(MemoryWorkflowStore::new());
    let seed_factory = Arc::new(FakeFactory::new(false));
    let seed_service = live_service(
        Arc::clone(&seed_store) as Arc<dyn WorkflowStore>,
        Arc::clone(&seed_factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("seed service");
    let actor = actor();
    let request = request("request-live-store-failure", definition(false));
    let seeded = seed_service
        .submit_run(&actor, request.clone())
        .expect("seed admission");
    let seeded_snapshot = seed_service
        .status(&actor, &seeded.workflow_run_id)
        .expect("seed status")
        .run;
    let seeded_events = seed_store
        .events(&seeded.workflow_run_id, 0, 128)
        .expect("seed events")
        .events;
    seed_service
        .command(
            &actor,
            command(
                &seeded.workflow_run_id,
                "seed-cleanup",
                seeded.run_revision,
                CommandKind::Cancel,
            ),
        )
        .expect("seed cleanup");

    // Re-home the seeded run under a different request id. The submission below
    // recomputes the same live run identity from the same definition, so the
    // durable reservation collides with the run already retained here.
    let seeded_run_id = seeded_snapshot.workflow_run_id.clone();
    let store = Arc::new(MemoryWorkflowStore::new());
    store
        .create_run(
            "request-live-store-conflict",
            "fixture-store-conflict",
            seeded_snapshot.clone(),
            seeded_events,
        )
        .expect("seed duplicate run");
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_service(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    // The reservation boundary rejects the colliding identity while writing the
    // durable snapshot (`crates/harness/src/management/store_ops_run.rs:24-29`
    // returns `duplicate_run`), which is reached from
    // `crates/harness/src/management/execution.rs:250` before `factory.open`
    // (`:253`) and `session.launch` (`:255`). No live session is ever opened, so
    // no live run can leak, and the refusal is not a silent success.
    let error = service
        .submit_run(&actor, request.clone())
        .expect_err("duplicate identity must be refused");
    assert_eq!(error.code, "duplicate_run");
    assert!(factory.entries().is_empty());
    // The refused submission leaves the retained run byte-identical, so the
    // refusal is never recorded as a silent success.
    let retained = store
        .get_run(&seeded_run_id)
        .expect("durable read")
        .expect("retained run");
    assert_eq!(retained, seeded_snapshot);

    // A retry reaches the same boundary and is refused identically: the failed
    // submission is never retained as a successful one.
    let retry_error = service
        .submit_run(&actor, request)
        .expect_err("duplicate identity remains refused");
    assert_eq!(retry_error.code, "duplicate_run");
    assert!(factory.entries().is_empty());
}

#[test]
fn unavailable_authored_binding_is_rejected_before_live_launch() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_service(
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
    let service = live_service(
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
    let service = live_service(
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
fn unresolved_cancel_keeps_pending_identity_and_live_session() {
    let factory = Arc::new(FakeFactory::unresolved_reconcile());
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-live-cancel-unknown", definition(false)),
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
    let before = service.status(&actor, &run_id).expect("status").run;
    let operation_id = before
        .pending_operation
        .as_ref()
        .expect("pending operation")
        .operation_id
        .clone();

    let cancelled = service
        .command(&actor, command(&run_id, "cancel", 4, CommandKind::Cancel))
        .expect("cancel remains pending");
    assert_eq!(
        cancelled.outcome,
        sts2_harness::management::CommandOutcome::Pending
    );
    let after = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(after.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(
        after
            .pending_operation
            .as_ref()
            .expect("pending operation retained")
            .operation_id,
        operation_id
    );
    assert_eq!(
        after.cleanup,
        sts2_harness::management::CleanupState::NotStarted
    );
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
fn cancel_consumes_a_reconciled_pending_operation_before_cleanup() {
    let factory = Arc::new(FakeFactory::new(true));
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-live-cancel-resolved", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    for (id, revision) in [("observe", 1), ("decide", 2), ("action", 3)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }
    let cancelled = service
        .command(&actor, command(&run_id, "cancel", 4, CommandKind::Cancel))
        .expect("cancel");
    assert_eq!(
        cancelled.outcome,
        sts2_harness::management::CommandOutcome::Applied
    );
    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::Cancelled);
    assert!(snapshot.pending_operation.is_none());
    assert!(factory.entries().contains(&"reconcile".to_owned()));
}

#[test]
fn launch_failure_releases_partial_session() {
    let factory = Arc::new(FakeFactory::launch_error());
    let service = live_service(
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
    let service = live_service(
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

#[test]
fn unavailable_context_owner_fails_live_admission_before_launch() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_store(
        Arc::clone(&store) as Arc<dyn sts2_harness::management::WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    assert!(!service.context_owner_port().is_available());
    let error = service
        .submit_run(
            &actor(),
            request("request-live-owner-missing", definition(false)),
        )
        .expect_err("missing owner must fail closed");
    assert_eq!(error.code, "context_owner_unavailable");
    assert!(factory.entries().is_empty());
}
