// SPDX-License-Identifier: MIT

//! A Studio consumer reading a run back must see which composition served it.
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, ExecutionMode, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementServer,
    RunRequest, ServerConfig, SqliteWorkflowStore, StaticAuthenticator, synthetic_sqlite_store,
};

/// `management/cli.rs` `serve` composes exactly `synthetic_sqlite_store`. The
/// run snapshot it serves must therefore report the synthetic execution mode so
/// a labelled fixture run cannot be mistaken for live gateway execution.
#[test]
fn served_synthetic_runs_report_the_synthetic_execution_mode()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("sts2-run-mode-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let store = SqliteWorkflowStore::open(directory.join("store.sqlite"))?;
    let service = Arc::new(synthetic_sqlite_store(Arc::new(store)));
    let operator = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let authenticator = StaticAuthenticator::new().with_credential("operator-token", operator)?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, service)?;
    let client = ManagementClient::new(server.address(), "operator-token")?;

    let definition: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-run-mode".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "sts2-synthetic-1".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    };
    let response = client.request_json(
        "POST",
        "/v1/workflow-runs",
        Some(&serde_json::to_vec(&request)?),
    )?;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let submitted: serde_json::Value = serde_json::from_slice(&response.body)?;
    let run_id = submitted["workflow_run_id"].as_str().expect("run id");
    let path = format!("/v1/workflow-runs/{run_id}");
    let status: serde_json::Value =
        serde_json::from_slice(&client.request_json("GET", &path, None)?.body)?;
    assert_eq!(
        status["run"]["execution_mode"],
        serde_json::to_value(ExecutionMode::Synthetic)?
    );

    server.shutdown()?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
