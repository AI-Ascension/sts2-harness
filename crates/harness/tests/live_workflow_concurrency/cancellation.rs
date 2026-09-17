// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use sts2_harness::management::{
    CommandKind, CommandOutcome, PendingOperationState, WorkflowRunStatus,
};

use super::{GateRelease, SessionGate, command, command_with_timeout, gated_service};
use crate::support::{actor, definition, request};

#[test]
fn a_busy_live_run_does_not_block_another_runs_command() {
    let gate = Arc::new(SessionGate::new("observe"));
    let (service, factory) = gated_service(&gate, false);
    let actor = actor();
    let first = service
        .submit_run(
            &actor,
            request("request-live-independent-first", definition(false)),
        )
        .expect("submit first");
    let second = service
        .submit_run(
            &actor,
            request("request-live-independent-second", definition(false)),
        )
        .expect("submit second");

    let stepping = {
        let service = Arc::clone(&service);
        let actor = actor.clone();
        let run_id = first.workflow_run_id.clone();
        thread::spawn(move || {
            service.command(&actor, command(&run_id, "first-step", 1, CommandKind::Step))
        })
    };
    let _release = GateRelease(Arc::clone(&gate));
    assert!(
        gate.wait_entered(Duration::from_secs(10)),
        "first worker did not reach the gated observe port"
    );

    // The registry is not held through the first run's port call. The second
    // run can acquire only its own state and complete its independent step.
    let independent = command_with_timeout(
        &service,
        &actor,
        command(&second.workflow_run_id, "second-step", 1, CommandKind::Step),
        Duration::from_secs(10),
    )
    .expect("independent command");
    assert_eq!(independent.outcome, CommandOutcome::Applied);
    assert_eq!(independent.run_revision, 2);

    gate.release();
    let blocked = stepping.join().expect("join").expect("first command");
    assert_eq!(blocked.outcome, CommandOutcome::Applied);
    assert_eq!(blocked.run_revision, 2);
    assert_eq!(
        factory
            .entries()
            .iter()
            .filter(|entry| entry.as_str() == "observe")
            .count(),
        2
    );
}

#[test]
fn cancellation_during_an_in_flight_step_is_deferred_then_dominates() {
    let gate = Arc::new(SessionGate::new("observe"));
    let (service, factory) = gated_service(&gate, false);
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-live-cancel-race", definition(false)),
        )
        .expect("submit");
    let run_id = submitted.workflow_run_id;

    let stepping = {
        let service = Arc::clone(&service);
        let actor = actor.clone();
        let run_id = run_id.clone();
        thread::spawn(move || {
            service.command(
                &actor,
                command(&run_id, "step-in-flight", 1, CommandKind::Step),
            )
        })
    };
    let _release = GateRelease(Arc::clone(&gate));
    assert!(
        gate.wait_entered(Duration::from_secs(10)),
        "worker did not reach the gated observe port"
    );

    // Cancellation does not race an operation that is already durably in
    // flight; it is acknowledged as pending at the pre-step revision and is not
    // persisted (the store returns Pending without queueing the competitor).
    let deferred = command_with_timeout(
        &service,
        &actor,
        command(&run_id, "cancel-in-flight", 1, CommandKind::Cancel),
        Duration::from_secs(10),
    )
    .expect("cancel while in flight");
    assert_eq!(deferred.outcome, CommandOutcome::Pending);
    assert_eq!(deferred.run_revision, 1);
    assert_eq!(deferred.sequence, None);

    gate.release();
    let stepped = stepping.join().expect("join").expect("step");
    assert_eq!(stepped.outcome, CommandOutcome::Applied);
    assert_eq!(stepped.run_revision, 2);

    // The deferred cancellation did not silently take effect: the run is still
    // Running and no cleanup was performed once the in-flight step settled.
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Running
    );
    let settled = factory.entries();
    assert!(
        !settled
            .iter()
            .any(|entry| entry == "stop" || entry == "release"),
        "no cleanup may run before an explicit cancellation: {settled:?}"
    );

    // A fresh cancellation at the refreshed revision applies and stops the
    // live session. (The earlier competing request was never persisted.)
    let cancelled = service
        .command(
            &actor,
            command(&run_id, "cancel-after-step", 2, CommandKind::Cancel),
        )
        .expect("cancel");
    assert_eq!(cancelled.outcome, CommandOutcome::Applied);
    assert_eq!(cancelled.run_revision, 3);
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Cancelled
    );
    assert!(factory.entries().contains(&"stop".to_owned()));

    // Cancellation dominates a later step: the complete port log is unchanged,
    // so no further node body or cleanup effect executes.
    let before_dominated = factory.entries();
    let dominated = service
        .command(
            &actor,
            command(&run_id, "step-after-cancel", 3, CommandKind::Step),
        )
        .expect("step after cancel");
    assert_eq!(dominated.outcome, CommandOutcome::Applied);
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Cancelled
    );
    assert_eq!(
        factory.entries(),
        before_dominated,
        "a cancelled run must not execute any further port effect"
    );
    assert_eq!(
        factory
            .entries()
            .iter()
            .filter(|entry| entry.as_str() == "observe")
            .count(),
        1
    );
}

#[test]
fn cancel_after_an_accepted_barrier_timeout_reconciles_the_same_operation() {
    let gate = Arc::new(SessionGate::new("wait"));
    let (service, factory) = gated_service(&gate, true);
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-live-cancel-accepted-barrier", definition(false)),
        )
        .expect("submit");
    let run_id = submitted.workflow_run_id;
    for (id, revision) in [("observe", 1), ("decide", 2)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("prepare action");
    }

    let stepping = {
        let service = Arc::clone(&service);
        let actor = actor.clone();
        let run_id = run_id.clone();
        thread::spawn(move || {
            service.command(
                &actor,
                command(&run_id, "accepted-action", 3, CommandKind::Step),
            )
        })
    };
    let _release = GateRelease(Arc::clone(&gate));
    assert!(
        gate.wait_entered(Duration::from_secs(10)),
        "accepted action did not reach its settlement barrier"
    );

    // A concurrent cancellation remains a durable-command fence response; it
    // cannot run cleanup against a session whose accepted action is still at
    // the barrier.
    let deferred = command_with_timeout(
        &service,
        &actor,
        command(&run_id, "cancel-during-barrier", 3, CommandKind::Cancel),
        Duration::from_secs(10),
    )
    .expect("cancel while accepted action is in flight");
    assert_eq!(deferred.outcome, CommandOutcome::Pending);
    assert_eq!(deferred.run_revision, 3);
    assert_eq!(deferred.sequence, None);

    gate.release();
    let pending = stepping.join().expect("join").expect("accepted action");
    assert_eq!(pending.outcome, CommandOutcome::Pending);
    assert_eq!(pending.run_revision, 4);
    let pending_operation = service
        .status(&actor, &run_id)
        .expect("status")
        .run
        .pending_operation
        .expect("accepted operation retained for reconciliation");
    assert_eq!(
        pending_operation.classification,
        PendingOperationState::Accepted
    );

    // A cancellation at the refreshed revision reconciles the retained
    // operation identity before it stops and releases the live session.
    let cancelled = service
        .command(
            &actor,
            command(&run_id, "cancel-after-barrier", 4, CommandKind::Cancel),
        )
        .expect("cancel after reconciliation");
    assert_eq!(cancelled.outcome, CommandOutcome::Applied);
    assert_eq!(cancelled.run_revision, 5);
    assert_eq!(
        service.status(&actor, &run_id).expect("status").run.status,
        WorkflowRunStatus::Cancelled
    );
    assert_eq!(
        factory.entries(),
        [
            "launch",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "reconcile",
            "stop",
            "release"
        ]
    );
}
