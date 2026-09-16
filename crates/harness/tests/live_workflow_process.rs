// SPDX-License-Identifier: MIT

//! Real HTTP and process boundaries with recording synthetic session/context ports.
//! This does not launch the production CLI, native game, gateway, or provider.
#![allow(clippy::expect_used)]

use std::io::BufRead;
use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    CommandKind, LiveWorkflowOptions, ManagementClient, ManagementServer, MemoryWorkflowStore,
    ServerConfig, StaticAuthenticator, digest_value,
};

#[path = "support/live_workflow_process.rs"]
mod process;
#[path = "support/live_workflow.rs"]
mod support;

#[test]
#[ignore = "launched explicitly by authenticated_process_executes_authored_graph"]
fn recording_server_worker() {
    assert_eq!(
        std::env::var("STS2_RECORDING_PROCESS_TEST").as_deref(),
        Ok("1")
    );
    let factory = Arc::new(support::FakeFactory::new(false));
    let service = support::live_service(
        Arc::new(MemoryWorkflowStore::new()),
        factory.clone(),
        LiveWorkflowOptions::default(),
    )
    .expect("live store service");
    let auth = StaticAuthenticator::single("recording-test-token", support::actor())
        .expect("authenticator");
    let config = ServerConfig::new("127.0.0.1:0".parse().expect("loopback"), Arc::new(auth))
        .expect("server config");
    let server = ManagementServer::start(config, Arc::new(service)).expect("server");
    process::emit(json!({"address": server.address().to_string(), "pid": std::process::id()}));
    for line in std::io::stdin().lock().lines() {
        match line.expect("control input").as_str() {
            "snapshot" => process::emit(json!({
                "entries": factory.entries(),
                "launches": factory.launches().iter().map(|launch| json!({
                    "digest": launch.definition_digest, "nodes": launch.node_order
                })).collect::<Vec<_>>()
            })),
            "shutdown" => break,
            other => assert_eq!(other, "snapshot", "unknown control message"),
        }
    }
    server.shutdown().expect("server shutdown");
}

fn http(
    client: &ManagementClient,
    method: &str,
    path: &str,
    body: Option<Value>,
    status: u16,
) -> Value {
    let bytes = body.map(|body| serde_json::to_vec(&body).expect("request JSON"));
    let response = client
        .request_json(method, path, bytes.as_deref())
        .expect("HTTP response");
    assert_eq!(
        response.status,
        status,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).expect("response JSON")
}

#[test]
fn authenticated_process_executes_authored_graph() {
    let mut worker = process::Worker::start();
    let empty = worker.evidence();
    assert_eq!(empty, json!({"entries": [], "launches": []}));
    let unauthorized = serde_json::to_value(support::request("denied", support::definition(false)))
        .expect("request");
    http(
        &worker.unauthorized,
        "POST",
        "/v1/workflow-runs",
        Some(unauthorized),
        401,
    );
    assert_eq!(
        worker.evidence(),
        empty,
        "unauthorized submission has no port effects"
    );

    for alternate in [false, true] {
        execute_graph(&mut worker, alternate);
    }
    worker.finish();
}

fn authored_definition(alternate: bool) -> Value {
    let mut definition = support::definition(false);
    if alternate {
        // Insert a second observation on the authored success path.
        let graph = &mut definition["graphs"][0];
        let mut extra = graph["nodes"][0].clone();
        extra["id"] = json!("observe-again");
        graph["nodes"].as_array_mut().expect("nodes").push(extra);
        graph["edges"][0]["to"] = json!("observe-again");
        graph["edges"]
            .as_array_mut()
            .expect("edges")
            .push(json!({"from": "observe-again", "to": "decide", "on": "ok", "priority": 0}));
    }
    definition
}

fn execute_graph(worker: &mut process::Worker, alternate: bool) {
    let definition = authored_definition(alternate);
    let digest = digest_value(&definition).expect("digest");
    let nodes = definition["graphs"][0]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|node| node["id"].clone())
        .collect::<Vec<_>>();
    let before = worker.evidence()["entries"]
        .as_array()
        .expect("entries")
        .len();
    let request = support::request(if alternate { "alternate" } else { "original" }, definition);
    let submitted = http(
        &worker.client,
        "POST",
        "/v1/workflow-runs",
        Some(serde_json::to_value(request).expect("request")),
        200,
    );
    let run_id = submitted["workflow_run_id"].as_str().expect("run id");
    assert_eq!(submitted["status"], "running");
    let path = format!("/v1/workflow-runs/{run_id}");
    let launches = worker.evidence()["launches"].clone();
    assert_eq!(
        launches
            .as_array()
            .expect("launches")
            .last()
            .expect("launch"),
        &json!({"digest": digest, "nodes": nodes})
    );
    reject_command(worker, run_id, &path);
    step_graph(worker, run_id, &path, alternate, before);
    assert_completed(worker, &path, &digest, alternate, before);
}

fn assert_completed(
    worker: &mut process::Worker,
    path: &str,
    digest: &str,
    alternate: bool,
    before: usize,
) {
    let status = http(&worker.client, "GET", path, None, 200);
    assert_eq!(status["run"]["status"], "completed");
    assert_eq!(status["run"]["execution_mode"], "live");
    assert_eq!(status["run"]["definition_digest"], digest);
    let expected = if alternate {
        json!([
            "launch",
            "observe",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "wait",
            "release"
        ])
    } else {
        json!([
            "launch",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "wait",
            "release"
        ])
    };
    assert_eq!(
        &worker.evidence()["entries"].as_array().expect("entries")[before..],
        expected.as_array().expect("expected")
    );
}

fn reject_command(worker: &mut process::Worker, run_id: &str, path: &str) {
    let prior = worker.evidence();
    let denied = support::command(run_id, "denied-step", 1, CommandKind::Step);
    http(
        &worker.unauthorized,
        "POST",
        &format!("{path}/commands"),
        Some(serde_json::to_value(denied).expect("command")),
        401,
    );
    assert_eq!(
        worker.evidence(),
        prior,
        "unauthorized command has no port effects"
    );
}

fn step_graph(
    worker: &mut process::Worker,
    run_id: &str,
    path: &str,
    alternate: bool,
    before: usize,
) {
    let count = if alternate { 5 } else { 4 };
    for revision in 1..=count {
        let command = support::command(
            run_id,
            &format!("step-{alternate}-{revision}"),
            revision,
            CommandKind::Step,
        );
        let response = http(
            &worker.client,
            "POST",
            &format!("{path}/commands"),
            Some(serde_json::to_value(command).expect("command")),
            200,
        );
        assert_eq!(response["outcome"], "applied");
        assert_eq!(response["run_revision"], revision + 1);
        if !alternate && revision == 3 {
            let evidence = worker.evidence();
            assert_eq!(
                &evidence["entries"].as_array().expect("entries")[before..],
                json!([
                    "launch",
                    "observe",
                    "legal_actions",
                    "decide",
                    "dispatch",
                    "wait"
                ])
                .as_array()
                .expect("expected")
            );
            let status = http(&worker.client, "GET", path, None, 200);
            assert_eq!(status["run"]["cursor"]["node_id"], "done");
            assert!(status["run"]["pending_operation"].is_null());
            assert_eq!(status["run"]["status"], "running");
        }
    }
}
