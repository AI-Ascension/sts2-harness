// SPDX-License-Identifier: MIT

//! Held `Decide` attempts at the served boundary.
//!
//! A live `Decide` node pays one model runtime exchange. These tests pin the served behaviour that
//! makes a lost reply, a re-issued operator step, a changed catalog and a restart unable to turn
//! that one paid exchange into a second one, and that keeps the outstanding attempt visible to
//! recovery admission instead of looks-safely-resumable.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, CommandKind, CommandOutcome, CommandResponse, EventType, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, ManagementService, MemoryWorkflowStore, PendingOperationState,
    RecoveryAdmission, WorkflowRunStatus, WorkflowStore,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

fn live_factory(factory: &Arc<FakeFactory>) -> Arc<dyn LiveWorkflowSessionFactory> {
    Arc::clone(factory) as Arc<dyn LiveWorkflowSessionFactory>
}

fn decides(factory: &FakeFactory) -> usize {
    factory
        .entries()
        .into_iter()
        .filter(|entry| entry == "decide")
        .count()
}

/// The reason code the durable event for this exact command published.
fn reason_code(
    service: &ManagementService,
    actor: &AuthContext,
    run_id: &str,
    response: &CommandResponse,
) -> String {
    service
        .events(actor, run_id, 0, 128)
        .expect("events")
        .events
        .into_iter()
        .find(|event| event.sequence == response.sequence.expect("command sequence"))
        .expect("command event")
        .payload
        .reason_code
}

#[test]
fn lost_decision_reply_holds_the_attempt_and_a_later_step_pays_nothing_new() {
    let factory = Arc::new(FakeFactory::decide_unknown());
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        live_factory(&factory),
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-decide-lost-reply", definition(false)),
        )
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");

    let lost = service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("lost decision reply");
    assert_eq!(lost.outcome, CommandOutcome::Pending);
    assert_eq!(
        reason_code(&service, &actor, &run_id, &lost),
        "live_operation_unknown"
    );
    assert_eq!(decides(&factory), 1);

    let held = service.status(&actor, &run_id).expect("status");
    assert_eq!(held.run.status, WorkflowRunStatus::NeedsOperator);
    let operation = held
        .run
        .pending_operation
        .as_ref()
        .expect("held decision attempt is durably visible");
    assert_eq!(operation.classification, PendingOperationState::Intent);
    assert_eq!(operation.instance_id, "instance-1");
    assert_eq!(operation.original_generation, 0);
    assert_eq!(held.recovery_admission, RecoveryAdmission::Reconciling);
    assert_eq!(held.authority.recovery, "pending_effect_visible");

    // The intent was recorded before the exchange crossed the provider boundary, so the durable
    // evidence of the paid attempt exists even though no decision came back.
    let intents = service
        .events(&actor, &run_id, 0, 128)
        .expect("events")
        .events
        .into_iter()
        .filter(|event| event.event_type == EventType::OperationIntent)
        .collect::<Vec<_>>();
    assert_eq!(intents.len(), 1);
    assert_eq!(
        intents[0].payload.operation_id.as_deref(),
        Some(operation.operation_id.as_str())
    );

    // A re-issued operator step re-enters the blocked cursor, but the held attempt identity is
    // reused: the provider is never reached a second time.
    let replayed = service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("re-issued step on the held attempt");
    assert_eq!(replayed.outcome, CommandOutcome::Pending);
    assert_eq!(
        reason_code(&service, &actor, &run_id, &replayed),
        "live_operation_unknown"
    );
    assert_eq!(decides(&factory), 1);
    let after = service.status(&actor, &run_id).expect("status");
    assert_eq!(after.run.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(
        after
            .run
            .pending_operation
            .as_ref()
            .map(|operation| operation.operation_id.as_str()),
        Some(operation.operation_id.as_str())
    );
    assert_eq!(after.recovery_admission, RecoveryAdmission::Reconciling);
}

#[test]
fn held_decision_attempt_survives_a_restart_as_reconciling_not_resumable() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(FakeFactory::decide_unknown());
    let service = live_service(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        live_factory(&factory),
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let run_request = request("request-decide-lost-reply-restart", definition(false));
    let run_id = service
        .submit_run(&actor, run_request.clone())
        .expect("submit")
        .workflow_run_id;
    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");
    service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("lost decision reply");
    drop(service);

    // A fresh process only sees the durable snapshot. Before the held attempt existed this run
    // looked safely resumable, which is exactly how a paid exchange became a second one.
    let restarted_factory = Arc::new(FakeFactory::decide_unknown());
    let restarted = live_service(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        live_factory(&restarted_factory),
        LiveWorkflowOptions::default(),
    )
    .expect("restarted service");
    let status = restarted.status(&actor, &run_id).expect("status");
    assert_eq!(status.recovery_admission, RecoveryAdmission::Reconciling);
    assert_eq!(
        status
            .run
            .pending_operation
            .as_ref()
            .expect("held attempt")
            .classification,
        PendingOperationState::Intent
    );
    let refused = restarted
        .submit_run(&actor, run_request)
        .expect_err("an outstanding decision attempt must require recovery");
    assert_eq!(refused.code, "live_submission_recovery_required");
    assert!(restarted_factory.entries().is_empty());
}

#[test]
fn refused_decision_releases_the_held_attempt() {
    let factory = Arc::new(FakeFactory::decide_error());
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        live_factory(&factory),
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let run_id = service
        .submit_run(&actor, request("request-decide-refused", definition(false)))
        .expect("submit")
        .workflow_run_id;
    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");
    let refused = service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("refused decision");

    // The refusal was reported before the provider adapter could write, so nothing is in doubt and
    // the run is not left advertising an outstanding attempt.
    assert_eq!(refused.outcome, CommandOutcome::Applied);
    assert_eq!(
        reason_code(&service, &actor, &run_id, &refused),
        "live_execution_failed"
    );
    let status = service.status(&actor, &run_id).expect("status");
    assert_eq!(status.run.status, WorkflowRunStatus::Failed);
    assert!(status.run.pending_operation.is_none());
    assert_eq!(status.authority.recovery, "none");
    assert_ne!(status.recovery_admission, RecoveryAdmission::Reconciling);
}

#[test]
fn changed_catalog_never_retargets_a_held_attempt_onto_a_new_exchange() {
    let factory = Arc::new(FakeFactory::decide_unknown_with_catalog_drift());
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        live_factory(&factory),
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-decide-catalog-drift", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");
    service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("lost decision reply");
    let held_digest = service
        .status(&actor, &run_id)
        .expect("status")
        .run
        .pending_operation
        .as_ref()
        .expect("held attempt")
        .payload_digest
        .clone();

    // The host answered the retry with a different catalog. The held identity may not be
    // retargeted and no new identity may be minted, so the attempt stays held and no exchange is
    // made for the changed request.
    let drifted = service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("drifted retry");
    assert_eq!(drifted.outcome, CommandOutcome::Pending);
    assert_eq!(
        reason_code(&service, &actor, &run_id, &drifted),
        "live_operation_unknown"
    );
    assert_eq!(decides(&factory), 1);
    let status = service.status(&actor, &run_id).expect("status");
    assert_eq!(status.run.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(
        status
            .run
            .pending_operation
            .as_ref()
            .expect("held attempt")
            .payload_digest,
        held_digest
    );
    assert_eq!(status.recovery_admission, RecoveryAdmission::Reconciling);
}
