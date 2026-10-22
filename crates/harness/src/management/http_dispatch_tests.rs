// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::management::{
    AuthContext, MemoryWorkflowStore, StaticAuthenticator, SubmissionRecoveryState,
};
use std::time::Instant;

#[path = "http_dispatch_test_support.rs"]
mod support;
use support::*;

struct Fixture {
    server: ServerHandle,
    service: Arc<ManagementService>,
    actor: AuthContext,
    stall: ReleaseOnDrop,
    lines: Arc<Lines>,
}

fn fixture() -> Fixture {
    let actor = AuthContext::new("dispatch-owner", ["workflow:*".to_owned()]).expect("actor");
    let foreign = AuthContext::new("other-owner", ["workflow:*".to_owned()]).expect("foreign");
    let auth = StaticAuthenticator::single("owner-token", actor.clone())
        .expect("auth")
        .with_credential("foreign-token", foreign)
        .expect("foreign credential");
    let stall = Arc::new(Stall::default());
    let service = Arc::new(
        crate::management::synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_definition_port(Arc::new(StalledDefinition(Arc::clone(&stall)))),
    );
    let mut config =
        ServerConfig::new("127.0.0.1:0".parse().expect("address"), Arc::new(auth)).expect("config");
    config.limits.deadline = Duration::from_secs(2);
    config.limits.dispatch_deadline = Duration::from_millis(100);
    config.limits.max_connections = 2;
    let lines = Arc::new(Lines::default());
    config.lifecycle_log = RequestLifecycleLog::with_sink(lines.clone());
    let server = ManagementServer::start(config, Arc::clone(&service)).expect("server");
    Fixture {
        server,
        service,
        actor,
        stall: ReleaseOnDrop(stall),
        lines,
    }
}

fn recovered(fixture: &Fixture, request_id: &str) -> crate::management::SubmissionRecoveryResponse {
    fixture
        .service
        .submission_recovery(&fixture.actor, request_id)
        .expect("recover original claim")
}

#[test]
fn only_recovery_specific_http_errors_preserve_owner_class() {
    use crate::management::ErrorClass;
    for class in [ErrorClass::Store, ErrorClass::WriteStateUnknown] {
        let owner = ManagementError::new(class.clone(), "owner_code", "owner classification");
        let expected = serde_json::to_value(owner.error_response()).expect("owner response");
        let response =
            super::response::error_response(HttpError::from_management(owner), MAX_RESPONSE_BYTES)
                .expect("HTTP encoding");
        let actual: serde_json::Value = serde_json::from_slice(&response.body).expect("wire JSON");
        assert_eq!(response.status, 503);
        assert_eq!(actual, expected);
    }
}

#[test]
fn existing_http_v1_errors_keep_their_legacy_wire_class() {
    use crate::management::ErrorClass;
    for (class, code, status, wire_class) in [
        (
            ErrorClass::InvalidInput,
            "legacy_error",
            400,
            "invalid_input",
        ),
        (
            ErrorClass::Authentication,
            "legacy_error",
            401,
            "authentication",
        ),
        (ErrorClass::Forbidden, "legacy_error", 403, "forbidden"),
        (
            ErrorClass::Capability,
            "inference_profile_context_incompatible",
            409,
            "conflict",
        ),
        (ErrorClass::Conflict, "legacy_error", 409, "conflict"),
        (ErrorClass::Unresolved, "legacy_error", 409, "conflict"),
        (ErrorClass::Budget, "legacy_error", 409, "conflict"),
        (ErrorClass::Replay, "legacy_error", 422, "replay"),
        (ErrorClass::Unavailable, "legacy_error", 503, "unavailable"),
        (
            ErrorClass::Capability,
            "draft_not_found",
            404,
            "invalid_input",
        ),
    ] {
        let owner = ManagementError::new(class, code, "legacy error detail");
        let response =
            super::response::error_response(HttpError::from_management(owner), MAX_RESPONSE_BYTES)
                .expect("HTTP encoding");
        let actual: serde_json::Value = serde_json::from_slice(&response.body).expect("wire JSON");
        assert_eq!(response.status, status, "{code}");
        assert_eq!(
            actual,
            serde_json::json!({
                "schema_version": "ascension.management/v1",
                "error": {
                    "class": wire_class,
                    "code": code,
                    "message": "legacy error detail",
                },
            }),
            "{code}"
        );
    }
}

#[test]
fn stalled_submission_answers_then_settles_without_a_second_launch() {
    let fixture = fixture();
    let client = ManagementClient::new(fixture.server.address(), "owner-token").expect("client");
    let request = body("bounded-request");
    let started = Instant::now();
    let response = client
        .request_json("POST", "/v1/workflow-runs", Some(&request))
        .expect("timeout reaches peer");
    assert_eq!(response.status, 503);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "dispatch must not hold the socket indefinitely"
    );
    let response = json(response);
    assert_eq!(response["schema_version"], "ascension.management-error/v2");
    assert_eq!(response["error"]["class"], "write_state_unknown");
    assert_eq!(response["error"]["code"], "dispatch_deadline_exceeded");
    assert_eq!(fixture.stall.0.entered.load(Ordering::SeqCst), 1);

    // Capacity stays occupied by the original work. Recovery has its reserved slot.
    let duplicate = client
        .request_json("POST", "/v1/workflow-runs", Some(&request))
        .expect("capacity response");
    assert_eq!(
        json(duplicate)["error"]["code"],
        "dispatch_capacity_exhausted"
    );
    let pending = json(
        client
            .request_json("GET", "/v1/workflow-submissions/bounded-request", None)
            .expect("recovery"),
    );
    assert_eq!(pending["state"], "pending");
    assert_eq!(pending["workflow_run_id"], serde_json::Value::Null);

    let foreign =
        ManagementClient::new(fixture.server.address(), "foreign-token").expect("foreign client");
    let hidden = json(
        foreign
            .request_json("GET", "/v1/workflow-submissions/bounded-request", None)
            .expect("hidden"),
    );
    assert_eq!(hidden["state"], "missing");
    assert_eq!(hidden["response"], serde_json::Value::Null);
    let duplicate_direct = fixture
        .service
        .submit_run(&fixture.actor, decode_strict(&request).expect("request"))
        .expect_err("pending claim blocks a concurrent direct submission too");
    assert_eq!(
        duplicate_direct.class,
        crate::management::ErrorClass::WriteStateUnknown
    );
    assert_eq!(fixture.stall.0.entered.load(Ordering::SeqCst), 1);

    fixture.stall.0.release();
    let settle_deadline = Instant::now() + Duration::from_secs(3);
    let settled = loop {
        let response = recovered(&fixture, "bounded-request");
        if response.state != SubmissionRecoveryState::Pending {
            break response;
        }
        assert!(
            Instant::now() < settle_deadline,
            "late completion must persist"
        );
        thread::yield_now();
    };
    assert_eq!(settled.state, SubmissionRecoveryState::Submitted);
    let replayed = fixture
        .service
        .submit_run(&fixture.actor, decode_strict(&request).expect("request"))
        .expect("same identity replay");
    assert_eq!(Some(replayed.workflow_run_id), settled.workflow_run_id);
    assert_eq!(fixture.stall.0.entered.load(Ordering::SeqCst), 1);
    fixture.server.shutdown().expect("completed work joins");
    let lines = fixture.lines.0.lock().expect("lines").join("\n");
    assert!(lines.contains("dispatch_start") && lines.contains("budget_ms=100"));
    assert!(lines.contains("dispatch_timeout") && lines.contains("request_end"));
    assert!(!lines.contains("owner-token") && !lines.contains("bounded-request"));
}

#[test]
fn shutdown_reports_retained_work_without_claiming_cancellation() {
    let fixture = fixture();
    let client = ManagementClient::new(fixture.server.address(), "owner-token").expect("client");
    let response = client
        .request_json("POST", "/v1/workflow-runs", Some(&body("shutdown-request")))
        .expect("bounded response");
    assert_eq!(response.status, 503);
    let started = Instant::now();
    let error = fixture.server.shutdown().expect_err("work is still held");
    assert_eq!(error.code, "dispatch_shutdown_pending");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        fixture
            .service
            .submission_recovery(&fixture.actor, "shutdown-request")
            .expect("claim")
            .state,
        SubmissionRecoveryState::Pending
    );
    fixture.stall.0.release();
    // The original worker remains the sole owner even after transport shutdown.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if fixture
            .service
            .submission_recovery(&fixture.actor, "shutdown-request")
            .expect("claim")
            .state
            == SubmissionRecoveryState::Submitted
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "retained worker must finish its claim"
        );
        thread::yield_now();
    }
    assert_eq!(fixture.stall.0.entered.load(Ordering::SeqCst), 1);
}
