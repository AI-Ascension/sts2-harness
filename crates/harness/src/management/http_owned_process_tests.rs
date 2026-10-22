// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use serde_json::{Value, json};
use std::time::Instant;

#[path = "http_owned_test_process.rs"]
mod process;
use process::*;
#[path = "http_owned_failure_tests.rs"]
mod failures;
#[path = "http_owned_protocol_tests.rs"]
mod protocol;
#[path = "http_owned_test_service.rs"]
mod service;

fn body(request_id: &str) -> Vec<u8> {
    serde_json::to_vec(&service::live::request(
        request_id,
        service::live::definition(false),
    ))
    .expect("request")
}
fn value(response: ClientResponse) -> Value {
    serde_json::from_slice(&response.body).expect("response JSON")
}

#[allow(clippy::panic)]
pub(super) fn panic_helper() {
    panic!("private helper panic fixture must never reach stderr");
}

#[test]
fn isolated_http_entry() -> Result<(), Box<dyn std::error::Error>> {
    process::entry()
}

#[test]
fn a_stalled_write_is_terminated_and_recovery_never_opens_the_session_twice() {
    let mut fixture = Fixture::start(false);
    let backend = fixture.process("backend", 0);
    let supervisor = fixture.process("supervisor", 0);
    let started = Instant::now();
    let request = body("stall-write-once");
    let response = fixture.request("POST", "/v1/workflow-runs", Some(&request));
    assert_eq!(response.status, 503);
    assert!(started.elapsed() < Duration::from_secs(4));
    let response = value(response);
    assert_eq!(response["schema_version"], "ascension.management-error/v2");
    assert_eq!(response["error"]["class"], "write_state_unknown");
    assert_eq!(response["error"]["code"], "dispatch_backend_terminated");
    assert_exited(&backend);
    assert_eq!(
        service::recorded(&fixture.root, "opens"),
        ["stall-write-once"]
    );
    assert_eq!(service::recorded(&fixture.root, "descendant").len(), 1);
    let pending = value(fixture.request("GET", "/v1/workflow-submissions/stall-write-once", None));
    assert_eq!(pending["state"], "pending");
    assert!(
        pending["workflow_run_id"].is_string(),
        "durable reservation survives termination"
    );
    let replay = value(fixture.request("POST", "/v1/workflow-runs", Some(&request)));
    assert_eq!(replay["error"]["class"], "write_state_unknown");
    let mut changed: Value = serde_json::from_slice(&request).expect("request JSON");
    changed["admission"]["catalog_revision"] = json!("changed.catalog.v1");
    let changed = serde_json::to_vec(&changed).expect("changed request");
    let conflict = fixture.request("POST", "/v1/workflow-runs", Some(&changed));
    assert_eq!(conflict.status, 409);
    assert_eq!(value(conflict)["error"]["code"], "submission_conflict");
    let foreign =
        ManagementClient::new(fixture.address, "foreign-owned-token").expect("foreign client");
    let hidden = value(
        foreign
            .request_json("GET", "/v1/workflow-submissions/stall-write-once", None)
            .expect("hidden recovery"),
    );
    assert_eq!(hidden["state"], "missing");
    assert_eq!(hidden["workflow_run_id"], Value::Null);
    assert_eq!(
        service::recorded(&fixture.root, "opens"),
        ["stall-write-once"]
    );
    assert_eq!(service::recorded(&fixture.root, "backend").len(), 2);
    let diagnostic_loss = fixture.diagnostic_loss();
    fixture.stop();
    assert_exited(&supervisor);
    let logs = fixture.logs();
    assert!(logs.contains("budget_ms=250"));
    assert!(
        logs.contains("dispatch_timeout") || diagnostic_loss > 0,
        "retirement can lose a queued marker only with explicit diagnostic loss"
    );
    assert!(!logs.contains(TOKEN) && !logs.contains("stall-write-once"));
}

#[test]
fn a_read_only_post_stall_is_unavailable_and_backend_replacement_invalidates_other_live_sessions() {
    let mut fixture = Fixture::start(false);
    let healthy = value(fixture.request("POST", "/v1/workflow-runs", Some(&body("healthy-run"))));
    let run_id = healthy["workflow_run_id"]
        .as_str()
        .expect("healthy admitted run");
    let path = format!("/v1/workflow-runs/{run_id}");
    let before = value(fixture.request("GET", &path, None));
    assert_eq!(before["recovery_admission"]["kind"], "safely_resumable");
    let mut definition = service::live::definition(false);
    definition["annotations"]["summary"] = json!("owned test stall");
    let body = serde_json::to_vec(&json!({"schema_version":"ascension.management/v1", "definition":definition, "capabilities":{}})).expect("validate body");
    let backend = fixture.process("backend", 0);
    let response = fixture.request("POST", "/v1/workflow-definitions/validate", Some(&body));
    assert_eq!(response.status, 503);
    let response = value(response);
    assert_eq!(response["schema_version"], "ascension.management/v1");
    assert_eq!(response["error"]["class"], "unavailable");
    assert_eq!(response["error"]["code"], "dispatch_backend_terminated");
    assert_exited(&backend);
    let after = value(fixture.request("GET", &path, None));
    assert_eq!(after["run"]["workflow_run_id"], run_id);
    assert_eq!(after["recovery_admission"]["kind"], "needs_operator");
    assert_eq!(service::recorded(&fixture.root, "opens"), ["healthy-run"]);
    fixture.stop();
}
