// SPDX-License-Identifier: MIT

//! Authenticated HTTP projection of recorded context-owner bindings.
//! Synthetic owner and SQLite fixtures only; no provider or game is launched.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, CommandKind, LiveWorkflowOptions, ManagementClient, ManagementServer,
    ManagementService, RECORDED_CONTEXT_BINDING_VIEW_SCHEMA, ServerConfig, SqliteWorkflowStore,
    StaticAuthenticator, live_store,
};

#[path = "support/context_binding_history.rs"]
mod history_support;
#[path = "support/live_workflow.rs"]
mod support;

use history_support::{Database, RecordingOwner};

fn service(
    store: Arc<SqliteWorkflowStore>,
    factory: Arc<support::FakeFactory>,
    owner: Arc<RecordingOwner>,
    history: bool,
) -> ManagementService {
    let service = live_store(store, factory, LiveWorkflowOptions::default())
        .expect("live service")
        .with_context_owner_port(owner);
    if history {
        service
            .with_context_binding_history()
            .expect("SQLite history")
    } else {
        service
    }
}

struct Fixture {
    service: Arc<ManagementService>,
    run_id: String,
    recorded: sts2_harness::management::ContextOwnerBinding,
    command_id: String,
    run_revision: u64,
}

fn record_binding(name: &str, history: bool) -> Fixture {
    let dir = Database::new(name);
    let store = Arc::new(SqliteWorkflowStore::open(&dir.0).expect("open"));
    let factory = Arc::new(support::FakeFactory::new(false));
    let owner = Arc::new(RecordingOwner::default());
    let service = Arc::new(service(store, factory, owner.clone(), history));
    let actor = support::actor();
    let run_id = service
        .submit_run(
            &actor,
            support::request("http-binding", support::definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    service
        .command(
            &actor,
            support::command(&run_id, "observe", 1, CommandKind::Step),
        )
        .expect("observe");
    let decide = support::command(&run_id, "decide", 2, CommandKind::Step);
    let response = service.command(&actor, decide.clone()).expect("decide");
    let accepted = owner.0.lock().expect("accepted").clone();
    let recorded = accepted.first().cloned().expect("one accepted binding");
    Fixture {
        service,
        run_id,
        recorded,
        command_id: decide.command_id,
        run_revision: response.run_revision,
    }
}

fn read(
    fixture: &Fixture,
    token: &str,
    authenticator: StaticAuthenticator,
    path: &str,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback address"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server =
        ManagementServer::start(config, Arc::clone(&fixture.service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client.request_json("GET", path, None).expect("response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    (response.status, value)
}

fn authenticator() -> StaticAuthenticator {
    StaticAuthenticator::single("owner-token", support::actor()).expect("authenticator")
}

#[test]
fn recorded_binding_is_projected_over_http_for_its_own_subject() {
    let fixture = record_binding("owner-view", true);
    let path = format!(
        "/v1/workflow-runs/{}/executions/live.node.2/context-binding",
        fixture.run_id
    );
    let (status, value) = read(&fixture, "owner-token", authenticator(), &path);
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value.get("schema_version").and_then(Value::as_str),
        Some(RECORDED_CONTEXT_BINDING_VIEW_SCHEMA)
    );
    assert_eq!(
        value.get("command_id").and_then(Value::as_str),
        Some(fixture.command_id.as_str())
    );
    assert_eq!(
        value.get("run_revision").and_then(Value::as_u64),
        Some(fixture.run_revision)
    );
    let binding = value.get("binding").expect("binding projection");
    assert_eq!(
        binding.get("binding_id").and_then(Value::as_str),
        Some(fixture.recorded.binding_id.as_str())
    );
    assert_eq!(
        binding.get("binding_digest").and_then(Value::as_str),
        Some(fixture.recorded.binding_digest.as_str())
    );
    let text = value.to_string();
    assert!(
        value.get("subject").is_none(),
        "subject must not be projected"
    );
    assert!(!text.contains("\"subject\""), "subject key leaked: {text}");
}

#[test]
fn unrecorded_invocation_returns_a_precise_not_recorded_error() {
    let fixture = record_binding("absent", true);
    let path = format!(
        "/v1/workflow-runs/{}/executions/live.node.9/context-binding",
        fixture.run_id
    );
    let (status, value) = read(&fixture, "owner-token", authenticator(), &path);
    assert_eq!(status, 404, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_binding_not_recorded")
    );
}

#[test]
fn another_subject_cannot_read_this_history() {
    let fixture = record_binding("foreign", true);
    let authenticator = StaticAuthenticator::new()
        .with_credential("owner-token", support::actor())
        .expect("owner credential")
        .with_credential(
            "foreign-token",
            AuthContext::new("other.operator", ["workflow:*".to_owned()]).expect("actor"),
        )
        .expect("foreign credential");
    let path = format!(
        "/v1/workflow-runs/{}/executions/live.node.2/context-binding",
        fixture.run_id
    );
    let (status, value) = read(&fixture, "foreign-token", authenticator, &path);
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_history_subject")
    );
}

#[test]
fn missing_workflow_read_scope_is_forbidden() {
    let fixture = record_binding("scope", true);
    let authenticator = StaticAuthenticator::single(
        "limited-token",
        AuthContext::new("operator", ["context:read".to_owned()]).expect("actor"),
    )
    .expect("authenticator");
    let path = format!(
        "/v1/workflow-runs/{}/executions/live.node.2/context-binding",
        fixture.run_id
    );
    let (status, value) = read(&fixture, "limited-token", authenticator, &path);
    assert_eq!(status, 403, "body: {value}");
}

#[test]
fn history_disabled_is_reported_as_unavailable_not_as_absent() {
    let fixture = record_binding("disabled", false);
    let path = format!(
        "/v1/workflow-runs/{}/executions/live.node.2/context-binding",
        fixture.run_id
    );
    let (status, value) = read(&fixture, "owner-token", authenticator(), &path);
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_history_unavailable")
    );
}

#[test]
fn unrelated_subpaths_do_not_resolve() {
    let fixture = record_binding("routes", true);
    for suffix in ["context-bindings", "context_binding"] {
        let path = format!(
            "/v1/workflow-runs/{}/executions/live.node.2/{suffix}",
            fixture.run_id
        );
        let (status, value) = read(&fixture, "owner-token", authenticator(), &path);
        assert_eq!(status, 400, "path {suffix} body: {value}");
        assert_eq!(
            value.pointer("/error/code").and_then(Value::as_str),
            Some("route_not_found")
        );
    }
}
