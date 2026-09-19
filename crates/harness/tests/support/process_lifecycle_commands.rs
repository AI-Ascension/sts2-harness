// SPDX-License-Identifier: MIT

//! Command-body and HTTP-exchange helpers for the lifecycle boundary suite.
//!
//! These build one lifecycle command body and drive one management exchange.
//! They are separated from `process_lifecycle.rs` only to keep each module
//! inside the repository size bound; the fixture and its recording port stay
//! together there.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    LifecycleIntentStore, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementServer,
    ManagementService, MemoryWorkflowStore, RunRequest, ServerConfig, StaticAuthenticator,
    synthetic_store,
};

use super::{Answer, COMMAND_SCHEMA, RecordingPort, actor, definition};

/// One management HTTP exchange against a service.
pub(crate) fn http(
    service: &Arc<ManagementService>,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(StaticAuthenticator::single("token", actor()).expect("authenticator")),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server");
    let client = ManagementClient::new(server.address(), "token").expect("client");
    let response = client.request_json(method, path, body).expect("response");
    server.shutdown().expect("server shutdown");
    (
        response.status,
        serde_json::from_slice(&response.body).expect("json response"),
    )
}

/// A synthetic run with no admitted target binding, plus a recording port.
///
/// The run exists and is authorized, so a lifecycle command reaches the target
/// resolution step and must be refused there: there is no admitted instance to
/// act on, and the harness must not substitute the instance its port is
/// configured for.
pub(crate) fn unadmitted() -> (Arc<ManagementService>, Arc<RecordingPort>, String, u64) {
    let directory =
        std::env::temp_dir().join(format!("sts2-lifecycle-noadmit-{}", uuid::Uuid::new_v4()));
    create_private_directory(&directory).expect("private directory");
    let port = Arc::new(RecordingPort::new(Vec::new(), Answer::STARTING));
    let intents = Arc::new(Mutex::new(
        LifecycleIntentStore::open(directory.join("intents")).expect("intent store"),
    ));
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_process_lifecycle(port.clone(), intents);
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-lifecycle-unadmitted".to_owned(),
        definition: Some(definition(true)),
        artifact_id: None,
        instance_id: "fixture-instance".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    };
    let snapshot = service
        .submit_run(&actor(), request)
        .expect("synthetic run without an admitted binding");
    (
        Arc::new(service),
        port,
        snapshot.workflow_run_id,
        snapshot.run_revision,
    )
}

/// Encodes one lifecycle command body.
pub(crate) fn command_body(
    command_id: &str,
    run_id: &str,
    expected_revision: u64,
    operation_id: u64,
    authority_epoch: u64,
    action: Value,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": COMMAND_SCHEMA,
        "command_id": command_id,
        "run_id": run_id,
        "expected_revision": expected_revision,
        "actor_scope": "operator",
        "operation_id": operation_id,
        "authority_epoch": authority_epoch,
        "action": action,
    }))
    .expect("command body")
}

/// The stable error code of a management error body.
pub(crate) fn error_code(value: &Value) -> Option<&str> {
    value.pointer("/error/code").and_then(Value::as_str)
}

/// A launch of one approved profile.
pub(crate) fn launch(profile_id: u64) -> Value {
    json!({"kind": "launch_new", "profile_id": profile_id})
}

/// A graceful stop.
pub(crate) fn stop() -> Value {
    json!({"kind": "stop", "mode": "graceful"})
}

/// Creates a directory only its owner can read.
pub(crate) fn create_private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}
