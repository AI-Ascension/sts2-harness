// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn run_graph_case(
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
    let expert_states = validate_graph_ledger(&ledger)?;
    Ok((value.0, value.1, expert_states))
}

fn validate_graph_ledger(ledger: &DownstreamLedger) -> Result<usize, Box<dyn std::error::Error>> {
    if !ledger.errors.is_empty() {
        return Err(format!("graph fixture failed: {:?}", ledger.errors).into());
    }
    let actions: Vec<_> = ledger
        .requests
        .iter()
        .enumerate()
        .filter(|(_, request)| request.path == "/api/v4/runtime/expert-action")
        .collect();
    if actions.len() != 1 {
        return Err(format!(
            "graph dispatched {} expert actions instead of exactly one",
            actions.len()
        )
        .into());
    }
    let (action_index, action) = actions[0];
    let operation_id = action.body["operation_id"]
        .as_str()
        .ok_or("graph action omitted its operation identity")?;
    if action.body["action"]["action_id"] != ACTION_ID {
        return Err("graph action did not preserve the selected action identity".into());
    }
    let settlement_path = format!("/api/v4/runtime/expert-actions/{operation_id}");
    let settlements: Vec<_> = ledger
        .requests
        .iter()
        .enumerate()
        .filter(|(_, request)| request.path == settlement_path)
        .collect();
    if settlements.is_empty() {
        return Err("graph completed without polling settlement for its action".into());
    }
    if settlements.iter().any(|(index, _)| *index <= action_index) {
        return Err("graph settlement poll did not follow the dispatched operation".into());
    }
    let settled = ledger
        .responses
        .iter()
        .filter(|response| {
            response.body["status"] == "settled"
                && response.body["operation_id"] == operation_id
                && response.body["action"]["action_id"] == ACTION_ID
        })
        .count();
    if settled == 0 {
        return Err("graph settlement poll never returned the dispatched operation".into());
    }
    Ok(ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-state")
        .count())
}

pub(super) fn add_observe_node(definition: &mut Value) -> Result<(), Box<dyn std::error::Error>> {
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
