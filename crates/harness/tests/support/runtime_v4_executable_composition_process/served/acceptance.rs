// SPDX-License-Identifier: MIT

//! End-to-end acceptance for the served live workflow boundary.
//!
//! The harness, gateway, and MCP peers are the real executable processes.  The
//! game-mod endpoint and provider bridge are deliberately bounded test doubles;
//! their ledgers make it possible to distinguish a rejected pre-provider
//! admission from a provider exchange or a dispatched game action.

use super::*;
use session::{
    WorkflowServiceConfig, response, served_definition, served_runtime_run_id_with,
    submit_policy_gate_with, wait_for_workflow_service, workflow_service_command,
};

const STALE_LEASE_ID: &str = "lease-stale";
const STALE_LEASE_EPOCH: u64 = 9;
const FOREIGN_INSTANCE_ID: &str = "instance-foreign";

pub(crate) fn run_served_peer_acceptance(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_negative_case(
        gateway_binary,
        mcp_binary,
        harness_binary,
        NegativeCase::WrongInstance,
    )?;
    run_negative_case(
        gateway_binary,
        mcp_binary,
        harness_binary,
        NegativeCase::StaleLease,
    )?;
    run_negative_case(
        gateway_binary,
        mcp_binary,
        harness_binary,
        NegativeCase::InvalidBinding,
    )?;
    run_negative_case(
        gateway_binary,
        mcp_binary,
        harness_binary,
        NegativeCase::UnavailableProvider,
    )?;
    run_graph_case(gateway_binary, mcp_binary, harness_binary)
}

#[derive(Clone, Copy)]
enum NegativeCase {
    WrongInstance,
    StaleLease,
    InvalidBinding,
    UnavailableProvider,
}

impl NegativeCase {
    fn label(self) -> &'static str {
        match self {
            Self::WrongInstance => "wrong-instance",
            Self::StaleLease => "stale-lease",
            Self::InvalidBinding => "invalid-binding",
            Self::UnavailableProvider => "unavailable-provider",
        }
    }
}

fn run_negative_case(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    case: NegativeCase,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let provider_capture = temporary.path.join(format!("{}.json", case.label()));
    let bridge = temporary.bridge_capturing(&provider_capture)?;
    let definition = served_definition()?;
    let instance_id = if matches!(case, NegativeCase::WrongInstance) {
        FOREIGN_INSTANCE_ID
    } else {
        INSTANCE_ID
    };
    let (lease_id, lease_epoch) = if matches!(case, NegativeCase::StaleLease) {
        (STALE_LEASE_ID, STALE_LEASE_EPOCH)
    } else {
        (LEASE_ID, LEASE_EPOCH)
    };
    let runtime_run_id = served_runtime_run_id_with(definition.clone(), instance_id, case.label())?;
    let policy_store = temporary.path.join("provider-policy.sqlite3");
    let context_store = temporary.path.join("context.sqlite3");
    let execution_store = temporary.path.join("execution.sqlite3");
    let workflow_store = temporary.path.join("workflow.sqlite3");
    seed_adopted_runtime_policy(&policy_store, &runtime_run_id)?;
    let invalid_context_config = if matches!(case, NegativeCase::InvalidBinding) {
        Some(serde_json::to_string(&json!({
            "schema_version":"ascension.workflow-context-owner-config.v1",
            "store_path":context_store,
            "key_reference":"STS2_SERVED_CONTEXT_OWNER_KEY",
            "owner_id":"served-context-owner",
            "owner_version":"v1",
            "context_ref":"context.invalid.v1",
            "limits":{"max_items":64,"max_notes":16,"max_context_bytes":131072,"max_objective_bytes":512,"max_control_events":64}
        }))?)
    } else {
        None
    };
    let missing_provider;
    let provider_path = if matches!(case, NegativeCase::UnavailableProvider) {
        missing_provider = temporary.path.join("provider-does-not-exist");
        missing_provider.as_path()
    } else {
        bridge.as_path()
    };
    let service_config = WorkflowServiceConfig {
        harness_binary,
        mcp_binary,
        bridge: provider_path,
        gateway_address: free_address()?,
        workflow_address: free_address()?,
        policy_store: &policy_store,
        context_store: &context_store,
        execution_store: &execution_store,
        workflow_store: &workflow_store,
        runtime_run_id: &runtime_run_id,
        context_owner_config: invalid_context_config.as_deref(),
        instance_id,
        lease_id,
        lease_epoch,
    };
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let mut gateway_process = if matches!(case, NegativeCase::StaleLease) {
        gateway_with_identity(
            gateway_binary,
            service_config.gateway_address,
            mod_server.address,
            INSTANCE_ID,
            lease_id,
            lease_epoch,
        )?
    } else {
        gateway(
            gateway_binary,
            service_config.gateway_address,
            mod_server.address,
        )?
    };
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway_process, service_config.gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let attempt: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, service_config.workflow_address)?;
            let error = match submit_policy_gate_with(
                &client,
                definition,
                instance_id,
                case.label(),
            ) {
                Ok(submission) if matches!(case, NegativeCase::UnavailableProvider) => {
                    let observe = CommandRequest {
                        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                        command_id: format!("{}-observe", case.label()),
                        run_id: submission.run_id.clone(),
                        expected_revision: submission.revision,
                        actor_scope: "profile:served".to_owned(),
                        kind: CommandKind::Step,
                        parameters: CommandParameters::default(),
                    };
                    let observed: CommandResponse = response(client.request_json(
                        "POST",
                        &format!("/v1/workflow-runs/{}/commands", submission.run_id),
                        Some(&serde_json::to_vec(&observe)?),
                    )?)?;
                    let decide = CommandRequest {
                        command_id: format!("{}-decide", case.label()),
                        expected_revision: observed.run_revision,
                        ..observe
                    };
                    let refused = client.request_json(
                        "POST",
                        &format!("/v1/workflow-runs/{}/commands", submission.run_id),
                        Some(&serde_json::to_vec(&decide)?),
                    )?;
                    let body = String::from_utf8_lossy(&refused.body).to_ascii_lowercase();
                    let status: Value = response(client.request_json(
                        "GET",
                        &format!("/v1/workflow-runs/{}", submission.run_id),
                        None,
                    )?)?;
                    if status["run"]["status"] != "failed" {
                        return Err(
                            format!("unavailable provider did not fail its run: {status}").into(),
                        );
                    }
                    let events = client.request_json(
                        "GET",
                        &format!(
                            "/v1/workflow-runs/{}/events?after_sequence=0&limit=128",
                            submission.run_id
                        ),
                        None,
                    )?;
                    let event_body = String::from_utf8_lossy(&events.body).to_ascii_lowercase();
                    if !body.contains("provider") && !event_body.contains("provider") {
                        return Err(format!(
                                "unavailable provider refusal lost its provider error: {body}; events={event_body}"
                            )
                            .into());
                    }
                    return Ok(());
                }
                Ok(_) => return Err("negative acceptance case unexpectedly submitted".into()),
                Err(error) => error.to_string(),
            };
            let lower = error.to_ascii_lowercase();
            let expected = match case {
                NegativeCase::WrongInstance => "gateway_allocate_failed",
                NegativeCase::StaleLease => "live_launch_fence_failed",
                NegativeCase::InvalidBinding => "context",
                NegativeCase::UnavailableProvider => "provider",
            };
            if !lower.contains(expected) {
                return Err(format!(
                    "{} did not preserve its truthful error identity: {error}",
                    case.label()
                )
                .into());
            }
            Ok(())
        })();
        let service_output = stop(service)?;
        attempt.map_err(|error| {
            format!(
                "{} failed: {error}; stdout={}; stderr={}",
                case.label(),
                String::from_utf8_lossy(&service_output.stdout),
                String::from_utf8_lossy(&service_output.stderr)
            )
        })?;
        assert_killed(&service_output, "negative served workflow")?;
        Ok(())
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    result?;
    if gateway_output.status.code() != Some(0) && !gateway_output.status.signal().is_some() {
        return Err(format!("{} gateway cleanup failed", case.label()).into());
    }
    if !ledger.errors.is_empty() {
        return Err(format!("{} fixture failed: {:?}", case.label(), ledger.errors).into());
    }
    if ledger
        .requests
        .iter()
        .any(|request| request.path == "/api/v4/runtime/expert-action")
        || ledger
            .requests
            .iter()
            .any(|request| request.path.starts_with("/api/v4/runtime/expert-actions/"))
    {
        return Err(format!("{} dispatched an action after rejection", case.label()).into());
    }
    let count_path = provider_capture.with_extension("count");
    if count_path.exists() {
        let count = std::fs::read_to_string(&count_path)?;
        if !count.is_empty() {
            return Err(format!(
                "{} exchanged with the provider after rejection ({} calls)",
                case.label(),
                count.len()
            )
            .into());
        }
    }
    Ok(())
}

fn run_graph_case(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let baseline = run_graph(gateway_binary, mcp_binary, harness_binary, false)?;
    let changed = run_graph(gateway_binary, mcp_binary, harness_binary, true)?;
    if baseline.0 == changed.0 {
        return Err("changed served graph retained the baseline digest".into());
    }
    let expected_baseline = ["observe", "decide", "execute", "execute", "done"];
    let expected_changed = [
        "observe", "observe2", "decide", "execute", "execute", "done",
    ];
    if baseline.1 != expected_baseline || changed.1 != expected_changed {
        return Err(format!(
            "changed served graph did not preserve authored node order: baseline={:?}, changed={:?}",
            baseline.1, changed.1
        )
        .into());
    }
    if changed.2 < baseline.2 + 1 {
        return Err(
            "changed served graph did not add an authoritative observation exchange".into(),
        );
    }
    Ok(())
}

/// Returns (definition digest, cursor order before each step, expert-state request count).
fn run_graph(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    changed: bool,
) -> Result<(String, Vec<String>, usize), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let capture = temporary.path.join("provider.json");
    let bridge = temporary.bridge_capturing(&capture)?;
    let mut definition = served_definition()?;
    if changed {
        add_observe_node(&mut definition)?;
    }
    let request_id = if changed {
        "graph-changed"
    } else {
        "graph-base"
    };
    let runtime_run_id = served_runtime_run_id_with(definition.clone(), INSTANCE_ID, request_id)?;
    let policy_store = temporary.path.join("provider-policy.sqlite3");
    let context_store = temporary.path.join("context.sqlite3");
    let execution_store = temporary.path.join("execution.sqlite3");
    let workflow_store = temporary.path.join("workflow.sqlite3");
    seed_adopted_runtime_policy(&policy_store, &runtime_run_id)?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let service_config = WorkflowServiceConfig {
        harness_binary,
        mcp_binary,
        bridge: &bridge,
        gateway_address,
        workflow_address,
        policy_store: &policy_store,
        context_store: &context_store,
        execution_store: &execution_store,
        workflow_store: &workflow_store,
        runtime_run_id: &runtime_run_id,
        context_owner_config: None,
        instance_id: INSTANCE_ID,
        lease_id: LEASE_ID,
        lease_epoch: LEASE_EPOCH,
    };
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let mut gateway_process = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<(String, Vec<String>), Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway_process, gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let attempt: Result<(String, Vec<String>), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submission = submit_policy_gate_with(&client, definition, INSTANCE_ID, request_id)?;
            let mut revision = submission.revision;
            let steps = if changed { 6 } else { 5 };
            let mut order = Vec::with_capacity(steps);
            for index in 1..=steps {
                let status: Value = response(client.request_json(
                    "GET",
                    &format!("/v1/workflow-runs/{}", submission.run_id),
                    None,
                )?)?;
                order.push(
                    status["run"]["cursor"]["node_id"]
                        .as_str()
                        .ok_or("graph cursor omitted node id")?
                        .to_owned(),
                );
                let command = CommandRequest {
                    schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                    command_id: format!("{request_id}-step-{index}"),
                    run_id: submission.run_id.clone(),
                    expected_revision: revision,
                    actor_scope: "profile:served".to_owned(),
                    kind: CommandKind::Step,
                    parameters: CommandParameters::default(),
                };
                let applied: CommandResponse = response(client.request_json(
                    "POST",
                    &format!("/v1/workflow-runs/{}/commands", submission.run_id),
                    Some(&serde_json::to_vec(&command)?),
                )?)?;
                if !matches!(
                    applied.outcome,
                    sts2_harness::management::CommandOutcome::Applied
                        | sts2_harness::management::CommandOutcome::Pending
                ) {
                    return Err(format!(
                        "graph step {index} was not applied: {:?}",
                        applied.outcome
                    )
                    .into());
                }
                revision = applied.run_revision;
            }
            let final_status: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{}", submission.run_id),
                None,
            )?)?;
            if final_status["run"]["status"] != "completed" {
                return Err(
                    format!("graph run did not settle before advancing: {final_status}").into(),
                );
            }
            let digest = final_status["run"]["definition_digest"]
                .as_str()
                .ok_or("graph run omitted definition digest")?
                .to_owned();
            Ok((digest, order))
        })();
        let service_output = stop(service)?;
        let value = attempt.map_err(|error| {
            format!(
                "graph workflow failed: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&service_output.stdout),
                String::from_utf8_lossy(&service_output.stderr)
            )
        })?;
        assert_killed(&service_output, "served graph workflow")?;
        Ok(value)
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    let value = result?;
    if gateway_output.status.code() != Some(0) && !gateway_output.status.signal().is_some() {
        return Err("graph gateway cleanup failed".into());
    }
    Ok((
        value.0,
        value.1,
        ledger
            .requests
            .iter()
            .filter(|request| request.path == "/api/v4/runtime/expert-state")
            .count(),
    ))
}

fn add_observe_node(definition: &mut Value) -> Result<(), Box<dyn std::error::Error>> {
    let graph = definition["graphs"]
        .as_array_mut()
        .and_then(|graphs| graphs.first_mut())
        .ok_or("served definition omitted graph")?;
    let observe = graph["nodes"]
        .as_array()
        .ok_or("served graph omitted nodes")?
        .iter()
        .find(|node| node["id"] == "observe")
        .cloned()
        .ok_or("served graph omitted observe node")?;
    let mut observe2 = observe;
    observe2["id"] = json!("observe2");
    graph["nodes"]
        .as_array_mut()
        .ok_or("served graph omitted nodes")?
        .push(observe2);
    let edges = graph["edges"]
        .as_array_mut()
        .ok_or("served graph omitted edges")?;
    for edge in edges.iter_mut() {
        if edge["from"] == "observe" && edge["to"] == "decide" && edge["on"] == "ok" {
            edge["to"] = json!("observe2");
        }
    }
    edges.push(json!({"from":"observe2","to":"decide","on":"ok","priority":0}));
    Ok(())
}
