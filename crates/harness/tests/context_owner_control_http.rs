// SPDX-License-Identifier: MIT

//! Owner control-command submission over authenticated management HTTP, and
//! the read-only companion token a served profile can mint. Synthetic owner and
//! in-memory store only; no provider or game is launched.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, Authenticator, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION, ContextControlCommand, ContextControlCommandKind,
    ContextControlReceipt, ContextOwnerPort, ContextSourceAdoptionRequest,
    EnvironmentAuthenticator, ManagementClient, ManagementServer, ManagementService, ServerConfig,
    StaticAuthenticator,
};

#[path = "support/context_owner_control_owner.rs"]
mod owner;

use owner::{ControlOwner, actor, boundary, error_code, run_id, service};

const MANIFEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn owner_service() -> (Arc<ControlOwner>, Arc<ManagementService>, String) {
    let owner = Arc::new(ControlOwner::default());
    let service = service(Arc::clone(&owner) as Arc<dyn ContextOwnerPort>);
    let run = run_id(&service);
    (owner, service, run)
}

fn owner_authenticator() -> Arc<dyn Authenticator> {
    Arc::new(StaticAuthenticator::single("owner-token", actor()).expect("authenticator"))
}

fn call(
    service: &Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, Value) {
    let config =
        ServerConfig::new("127.0.0.1:0".parse().expect("loopback"), authenticator).expect("config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client.request_json(method, path, body).expect("response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    (response.status, value)
}

fn submit(
    service: &Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    token: &str,
    run: &str,
    command: &ContextControlCommand,
) -> (u16, Value) {
    let body = serde_json::to_vec(command).expect("command body");
    call(
        service,
        authenticator,
        token,
        "POST",
        &format!("/v1/workflow-runs/{run}/context-control-commands"),
        Some(&body),
    )
}

fn receipt(value: &Value) -> ContextControlReceipt {
    serde_json::from_value(value.clone()).expect("v2 receipt")
}

fn pause(key: &str, expected_control_version: u64) -> ContextControlCommand {
    ContextControlCommand::Pause {
        idempotency_key: key.to_owned(),
        expected_control_version,
    }
}

#[test]
fn control_command_issues_owner_receipt_for_current_binding() {
    let (owner, service, run) = owner_service();
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &pause("control.pause.1", 1),
    );
    assert_eq!(status, 200, "body: {value}");
    let paused = receipt(&value);
    assert_eq!(paused.schema_version, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION);
    assert_eq!(paused.command, ContextControlCommandKind::Pause);
    assert_eq!(paused.effect, "pause_requested");
    assert_eq!(paused.binding_id, "test.binding.1");
    assert_eq!(paused.invocation_id, "test.invocation.1");
    assert_eq!(paused.idempotency_key, "control.pause.1");
    assert_eq!(paused.boundary.run_id, run);
    assert_eq!((paused.control_version, paused.gate_epoch), (2, 2));
    assert_eq!(paused.revision_id, None);

    let commit = ContextControlCommand::Commit {
        idempotency_key: "control.commit.1".to_owned(),
        expected_control_version: paused.boundary.control_version,
        expected_revision_id: "test.revision.1".to_owned(),
        expected_boundary: paused.boundary.clone(),
        preview_manifest_digest: MANIFEST.to_owned(),
        approved_manifest_digest: MANIFEST.to_owned(),
    };
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &commit,
    );
    assert_eq!(status, 200, "body: {value}");
    let committed = receipt(&value);
    assert_eq!(committed.command, ContextControlCommandKind::Commit);
    assert_eq!(committed.effect, "revision_committed");
    assert_eq!(committed.revision_id.as_deref(), Some("test.revision.2"));
    assert_eq!(committed.plan_epoch, 2);
    assert_eq!((committed.control_version, committed.gate_epoch), (3, 2));

    let resume = ContextControlCommand::Resume {
        idempotency_key: "control.resume.1".to_owned(),
        expected_control_version: committed.boundary.control_version,
        expected_boundary: committed.boundary.clone(),
    };
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &resume,
    );
    assert_eq!(status, 200, "body: {value}");
    let resumed = receipt(&value);
    assert_eq!(resumed.command, ContextControlCommandKind::Resume);
    assert_eq!(resumed.effect, "resume_accepted");
    assert_eq!((resumed.control_version, resumed.gate_epoch), (4, 3));
    assert_eq!(owner.effects(), 3);

    let (status, value) = call(
        &service,
        owner_authenticator(),
        "owner-token",
        "GET",
        &format!("/v1/workflow-runs/{run}/context-owner-association"),
        None,
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value
            .pointer("/binding/boundary/control_version")
            .and_then(Value::as_u64),
        Some(4),
        "the current association must reflect the owner's applied transitions"
    );
}

#[test]
fn control_command_with_stale_boundary_is_refused_without_receipt() {
    let (owner, service, run) = owner_service();
    let mut stale_boundary = boundary(&run);
    stale_boundary.gate_epoch = 7;
    let resume = ContextControlCommand::Resume {
        idempotency_key: "control.resume.stale".to_owned(),
        expected_control_version: 1,
        expected_boundary: stale_boundary.clone(),
    };
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &resume,
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_control_boundary_stale"));
    assert!(value.get("receipt").is_none() && value.get("boundary").is_none());

    let commit = ContextControlCommand::Commit {
        idempotency_key: "control.commit.stale".to_owned(),
        expected_control_version: 1,
        expected_revision_id: "test.revision.0".to_owned(),
        expected_boundary: boundary(&run),
        preview_manifest_digest: MANIFEST.to_owned(),
        approved_manifest_digest: MANIFEST.to_owned(),
    };
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &commit,
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_control_revision_stale"));

    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &pause("control.pause.stale", 9),
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_control_fence_stale"));
    assert_eq!(
        owner.effects(),
        0,
        "a stale command must never reach the owner"
    );
}

#[test]
fn control_command_requires_control_scope() {
    let (owner, service, run) = owner_service();
    let read_only: Arc<dyn Authenticator> = Arc::new(
        StaticAuthenticator::single(
            "read-token",
            AuthContext::new("integration.tester", ["workflow:read".to_owned()]).expect("actor"),
        )
        .expect("authenticator"),
    );
    let (status, value) = submit(
        &service,
        read_only,
        "read-token",
        &run,
        &pause("control.pause.1", 1),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("missing_scope"));
    assert_eq!(owner.effects(), 0);
}

#[test]
fn duplicate_idempotency_key_returns_original_receipt() {
    let (owner, service, run) = owner_service();
    let command = pause("control.pause.1", 1);
    let (status, first) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &command,
    );
    assert_eq!(status, 200, "body: {first}");
    let (status, second) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &command,
    );
    assert_eq!(status, 200, "body: {second}");
    assert_eq!(
        second, first,
        "a retried command must return the original receipt"
    );
    assert_eq!(
        owner.effects(),
        1,
        "a retried command must not apply a second effect"
    );

    // The same key with a different payload is not a retry: the harness finds no
    // recorded receipt for it and the owner refuses the conflicting reuse.
    let (status, value) = submit(
        &service,
        owner_authenticator(),
        "owner-token",
        &run,
        &pause("control.pause.1", 2),
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_owner_control_refused"));
    assert_eq!(owner.effects(), 1);
}

fn profile_lookup(name: &str) -> Option<String> {
    match name {
        "STS2_WORKFLOW_TOKEN_CONSOLE_LIVE" => Some("primary-token".to_owned()),
        "STS2_WORKFLOW_TOKEN_CONSOLE_LIVE_READ" => Some("read-token".to_owned()),
        _ => None,
    }
}

fn profile_authenticator() -> Arc<dyn Authenticator> {
    Arc::new(
        EnvironmentAuthenticator::from_profile_with("console-live", profile_lookup)
            .expect("profile authenticator"),
    )
}

#[test]
fn metadata_token_cannot_adopt_or_control() {
    let (owner, service, run) = owner_service();
    let (status, value) = call(
        &service,
        profile_authenticator(),
        "read-token",
        "GET",
        &format!("/v1/workflow-runs/{run}/context-owner-association"),
        None,
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value.pointer("/binding/binding_id").and_then(Value::as_str),
        Some("test.binding.1")
    );

    let adoption = ContextSourceAdoptionRequest {
        schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
        idempotency_key: "adopt.1".to_owned(),
        expected_control_version: 1,
        expected_revision_id: "test.revision.1".to_owned(),
        expected_boundary: boundary(&run),
    };
    let (status, value) = call(
        &service,
        profile_authenticator(),
        "read-token",
        "POST",
        &format!("/v1/workflow-runs/{run}/context-sources/strategy/adopt"),
        Some(&serde_json::to_vec(&adoption).expect("adoption body")),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("missing_scope"));

    let (status, value) = submit(
        &service,
        profile_authenticator(),
        "read-token",
        &run,
        &pause("control.pause.1", 1),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("missing_scope"));
    assert_eq!(owner.effects(), 0);

    let (status, value) = submit(
        &service,
        profile_authenticator(),
        "primary-token",
        &run,
        &pause("control.pause.1", 1),
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        owner.effects(),
        1,
        "the primary profile token keeps control"
    );
}
