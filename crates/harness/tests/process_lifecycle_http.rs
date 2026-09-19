// SPDX-License-Identifier: MIT

//! sts2-harness#101 boundary evidence over the real management HTTP surface.
//!
//! The gateway process-lifecycle owner is a recording synthetic port, because
//! the gateway advertises `available: false` today: `sts2-gateway` merged the
//! wire contract (ADR 0035) but no concrete OS adapter exists yet (ADR 0024).
//! Nothing here proves native launch, stop, or attach behaviour, and nothing
//! here proves gameplay readiness.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::management::{
    LaunchAcknowledgement, LaunchProfileId, LifecycleAction, LifecycleReadiness, ReadinessError,
    ReadinessObservation,
};

#[path = "support/process_lifecycle.rs"]
mod support;

use support::{
    Answer, Class, EPOCH, FOREIGN_INSTANCE, INSTANCE, command_body, error_code, harness, http,
    launch, stop, unadmitted,
};

fn digest(seed: u8) -> String {
    std::iter::repeat_n(format!("{seed:02x}"), 32).collect()
}

fn expected_code(answer: Answer) -> &'static str {
    match answer {
        Answer::Error { code, .. } => code,
        Answer::Operation(..) => panic!("only refusals are scripted in this case list"),
    }
}

#[test]
fn duplicate_launch_submits_once_and_preserves_authored_identity() {
    let fixture = harness(
        "request-lifecycle-duplicate",
        vec![Answer::STARTING],
        Answer::STARTING,
    );
    let body = fixture.command("cmd-launch", 41, EPOCH, launch(11));

    let (status, first) = fixture.http("POST", &fixture.operations_path(), Some(&body));
    assert_eq!(status, 200, "body: {first}");
    assert_eq!(first["classification"], "accepted");
    assert_eq!(first["operation_id"], 41);
    assert_eq!(first["instance_id"], INSTANCE);
    assert_eq!(first["gameplay_ready"], false);
    assert_eq!(first["state"], "Starting");
    assert_eq!(first["operation_state"], "Starting");
    assert_eq!(first["authority_epoch"], EPOCH);

    let (status, second) = fixture.http("POST", &fixture.operations_path(), Some(&body));
    assert_eq!(status, 200, "body: {second}");
    assert_eq!(
        second, first,
        "a duplicate request replays the settled answer"
    );

    // A duplicate under a *different* command identity is one operation too.
    let renamed = fixture.command("cmd-launch-again", 41, EPOCH, launch(11));
    let (status, third) = fixture.http("POST", &fixture.operations_path(), Some(&renamed));
    assert_eq!(status, 200, "body: {third}");
    assert_eq!(third["operation_id"], 41);
    assert_eq!(third["command_id"], "cmd-launch");

    let submits = fixture.port.submits();
    assert_eq!(submits.len(), 1, "exactly one launch was ever issued");
    assert_eq!(submits[0].command_id, "cmd-launch");
    assert_eq!(submits[0].operation_id, 41);
    assert_eq!(submits[0].authority_epoch, EPOCH);
    assert_eq!(submits[0].instance_id, INSTANCE);
    assert_eq!(
        submits[0].action,
        LifecycleAction::LaunchNew {
            profile_id: LaunchProfileId::new(11).expect("profile 11"),
        },
        "the authored action and approved profile reached the gateway unchanged"
    );
    assert!(fixture.port.lookups().is_empty());
}

#[test]
fn wrong_target_profile_epoch_and_unowned_attach_have_zero_downstream_effects() {
    let cases = [
        (
            "rejected-profile",
            launch(99),
            Answer::Error {
                class: Class::Invalid,
                code: "process_lifecycle_profile_rejected",
            },
            400,
        ),
        (
            "stale-epoch",
            launch(11),
            Answer::Error {
                class: Class::Conflict,
                code: "process_lifecycle_authority_epoch_stale",
            },
            409,
        ),
        (
            "unowned-attach",
            serde_json::json!({"kind": "attach_existing", "identity": {
                "instance_id": INSTANCE, "process": 7, "pid": 9, "birth_id": 3,
                "install_id": 1, "executable_id": 2, "image_id": 4, "namespace_id": 5
            }}),
            Answer::Error {
                class: Class::Forbidden,
                code: "process_lifecycle_unowned_attach",
            },
            403,
        ),
    ];
    for (label, action, answer, expected_status) in cases {
        let fixture = harness(
            &format!("request-lifecycle-{label}"),
            vec![answer],
            Answer::STARTING,
        );
        let body = fixture.command("cmd-refused", 61, EPOCH, action);
        let (status, value) = fixture.http("POST", &fixture.operations_path(), Some(&body));
        assert_eq!(status, expected_status, "{label}: body: {value}");
        assert_eq!(error_code(&value), Some(expected_code(answer)), "{label}");
        assert_eq!(
            fixture.port.submits().len(),
            1,
            "{label}: the refusal is not retried and nothing is re-issued"
        );
        assert!(fixture.port.lookups().is_empty(), "{label}: no reconcile");
        assert_eq!(
            fixture.port.seen_targets(),
            vec![INSTANCE.to_owned()],
            "{label}: only the admitted instance was ever addressed"
        );
        assert!(
            fixture
                .service
                .unresolved_lifecycle_operations()
                .expect("unresolved")
                .is_empty(),
            "{label}: a proven refusal is settled, not left outstanding"
        );
    }

    // A capability that names a foreign instance is refused before any submit.
    let fixture = harness(
        "request-lifecycle-capability-advert",
        Vec::new(),
        Answer::STARTING,
    );
    fixture.port.advertise(FOREIGN_INSTANCE);
    let (status, value) = fixture.http("GET", &fixture.capability_path(), None);
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("lifecycle_capability_scope_mismatch")
    );
    assert!(fixture.port.submits().is_empty());

    // No admitted binding means no instance at all: never a configured default.
    let (service, port, run_id, revision) = unadmitted();
    let body = command_body("cmd-unadmitted", &run_id, revision, 71, EPOCH, launch(11));
    let (status, value) = http(
        &service,
        "POST",
        &format!("/v1/workflow-runs/{run_id}/process-lifecycle/operations"),
        Some(&body),
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("lifecycle_target_not_admitted"));
    assert!(
        port.submits().is_empty(),
        "no fallback instance was addressed"
    );
    assert!(port.seen_targets().is_empty());
}

#[test]
fn lost_response_is_retained_unknown_and_reconciled_by_identity() {
    let fixture = harness(
        "request-lifecycle-lost",
        vec![Answer::TRANSPORT],
        Answer::STARTING,
    );
    let body = fixture.command("cmd-lost", 81, EPOCH, launch(11));
    let (status, value) = fixture.http("POST", &fixture.operations_path(), Some(&body));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("process_lifecycle_outcome_unknown")
    );

    let outstanding = fixture
        .service
        .unresolved_lifecycle_operations()
        .expect("unresolved");
    assert_eq!(outstanding.len(), 1);
    assert_eq!(outstanding[0].operation_id, 81);
    assert_eq!(outstanding[0].action_kind, "launch_new");
    assert_eq!(outstanding[0].authority_epoch, EPOCH);

    let (status, reconciled) = fixture.http(
        "POST",
        &format!("{}/81/reconcile", fixture.operations_path()),
        Some(b"{}"),
    );
    assert_eq!(status, 200, "body: {reconciled}");
    assert_eq!(reconciled["classification"], "accepted");
    assert_eq!(reconciled["operation_id"], 81);
    assert_eq!(reconciled["instance_id"], INSTANCE);
    assert_eq!(reconciled["state"], "Starting");
    assert_eq!(fixture.port.lookups(), vec![81]);
    assert_eq!(
        fixture.port.submits().len(),
        1,
        "reconciliation never re-issues the effect"
    );
    assert!(
        fixture
            .service
            .unresolved_lifecycle_operations()
            .expect("unresolved")
            .is_empty()
    );
}

#[test]
fn blocked_stop_and_cancelled_work_survive_restart_and_stop_dominates() {
    let fixture = harness(
        "request-lifecycle-restart",
        vec![Answer::BLOCKED, Answer::TRANSPORT],
        Answer::STOPPED,
    );
    let stop_body = fixture.command("cmd-stop", 91, EPOCH, stop());
    let (status, value) = fixture.http("POST", &fixture.operations_path(), Some(&stop_body));
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value["classification"], "unknown",
        "a blocked stop is unresolved"
    );
    assert_eq!(value["operation_state"], "Blocked");
    assert_eq!(value["gameplay_ready"], false);

    // A caller that gives up while the gateway answered nothing.
    let cancelled = fixture.command("cmd-cancelled", 92, EPOCH, launch(11));
    let (status, value) = fixture.http("POST", &fixture.operations_path(), Some(&cancelled));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("process_lifecycle_outcome_unknown")
    );

    let outstanding = |service: &std::sync::Arc<sts2_harness::management::ManagementService>| {
        service
            .unresolved_lifecycle_operations()
            .expect("unresolved")
            .iter()
            .map(|intent| intent.operation_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(outstanding(&fixture.service), vec![91, 92]);

    // Restart: a new service over the same store and a reopened intent journal.
    let restarted = fixture.restart();
    assert_eq!(
        outstanding(&restarted.service),
        vec![91, 92],
        "accepted and unknown work is preserved across a restart"
    );

    let (status, reconciled) = restarted.http(
        "POST",
        &format!("{}/91/reconcile", restarted.operations_path()),
        Some(b"{}"),
    );
    assert_eq!(status, 200, "body: {reconciled}");
    assert_eq!(reconciled["classification"], "stopped");
    assert_eq!(reconciled["operation_state"], "Stopped");
    assert_eq!(restarted.port.lookups(), vec![91]);
    assert_eq!(outstanding(&restarted.service), vec![92]);

    // A settled stop fences a later start: no second launch is issued.
    let later = restarted.command("cmd-after-stop", 93, EPOCH, launch(11));
    let (status, value) = restarted.http("POST", &restarted.operations_path(), Some(&later));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("process_lifecycle_stop_dominates"));
    assert_eq!(
        restarted.port.submits().len(),
        2,
        "the fenced launch never reached the gateway"
    );
}

#[test]
fn launch_acknowledgement_does_not_satisfy_a_gameplay_readiness_predicate() {
    let fixture = harness(
        "request-lifecycle-readiness",
        vec![Answer::STARTING],
        Answer::STARTING,
    );
    let body = fixture.command("cmd-launch", 101, EPOCH, launch(11));
    let (status, value) = fixture.http("POST", &fixture.operations_path(), Some(&body));
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(value["classification"], "accepted");
    assert_eq!(
        value["gameplay_ready"], false,
        "an accepted launch is not gameplay readiness"
    );

    // The acknowledgement is a real one, and it still cannot satisfy the
    // readiness predicate, which admits only readiness evidence.
    let acknowledgement = LaunchAcknowledgement {
        operation_id: 101,
        instance_id: INSTANCE.to_owned(),
        authority_epoch: EPOCH,
        started: true,
    };
    assert!(acknowledgement.started);
    let mut predicate = LifecycleReadiness::pending(INSTANCE, EPOCH);
    assert!(!predicate.is_satisfied());

    let foreign = ReadinessObservation::new(FOREIGN_INSTANCE, EPOCH, "obs-1", digest(0xab))
        .expect("observation");
    assert_eq!(
        predicate.satisfy(&foreign),
        Err(ReadinessError::InvalidObservation)
    );
    assert!(!predicate.is_satisfied());

    let observation =
        ReadinessObservation::new(INSTANCE, EPOCH, "obs-2", digest(0xcd)).expect("observation");
    predicate
        .satisfy(&observation)
        .expect("readiness evidence at the current epoch");
    assert!(predicate.is_satisfied());
}
