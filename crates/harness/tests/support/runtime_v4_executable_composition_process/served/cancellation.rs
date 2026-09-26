// SPDX-License-Identifier: MIT

use super::*;
use crate::fixture::ActionReadGate;
use std::sync::Arc;

struct GateRelease(Arc<ActionReadGate>);

impl Drop for GateRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Drives the served live path through an accepted action whose first
/// settlement read remains unresolved. The subsequent cancellation reconciles
/// the original operation before session cleanup. Gateway and MCP are real
/// peer processes; only the downstream mod endpoint and provider bridge are
/// synthetic fixtures.
pub(crate) fn run_served_cancel_after_accepted_barrier(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge()?;
    let (mod_server, gate) = ModServer::accepted_barrier_then_settled()?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let policy_store = temporary.path.join("served-provider-policy.sqlite3");
    let context_store = temporary.path.join("served-context.sqlite3");
    let execution_store = temporary.path.join("served-execution.sqlite3");
    let workflow_store = temporary.path.join("served-workflow.sqlite3");
    let runtime_run_id = served_runtime_run_id()?;
    seed_adopted_runtime_policy(&policy_store, &runtime_run_id)?;
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
    let mut gateway_process = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway_process, gateway_address)?;
        let mut command = workflow_service_command(&service_config)?;
        command
            .env("STS2_BARRIER_MAX_POLLS", "1")
            .env("STS2_BARRIER_WAIT_MILLIS", "1");
        let mut service = command.spawn()?;
        let attempt: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submitted = submit_and_step_policy_gate(&client, 2)?;
            let run_id = submitted.run_id.clone();
            let revision = submitted.revision;
            let action_client = ManagementClient::new(workflow_address, "served-workflow-token")?;
            let action = std::thread::spawn(move || -> Result<_, String> {
                let command = CommandRequest {
                    schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                    command_id: "served-accepted-action".to_owned(),
                    run_id: run_id.clone(),
                    expected_revision: revision,
                    actor_scope: "profile:served".to_owned(),
                    kind: CommandKind::Step,
                    parameters: CommandParameters::default(),
                };
                action_client
                    .request_json(
                        "POST",
                        &format!("/v1/workflow-runs/{run_id}/commands"),
                        Some(&serde_json::to_vec(&command).map_err(|error| error.to_string())?),
                    )
                    .map_err(|error| error.to_string())
            });
            let release = GateRelease(Arc::clone(&gate));
            if !gate.wait_entered(Duration::from_secs(5)) {
                return Err("accepted action did not enter its bounded barrier poll".into());
            }
            let deferred = CommandRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                command_id: "served-cancel-during-barrier".to_owned(),
                run_id: submitted.run_id.clone(),
                expected_revision: submitted.revision,
                actor_scope: "profile:served".to_owned(),
                kind: CommandKind::Cancel,
                parameters: CommandParameters::default(),
            };
            let deferred: CommandResponse = response(client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{}/commands", submitted.run_id),
                Some(&serde_json::to_vec(&deferred)?),
            )?)?;
            if deferred.outcome != sts2_harness::management::CommandOutcome::Pending
                || deferred.sequence.is_some()
            {
                return Err(
                    format!("concurrent cancel was unexpectedly accepted: {deferred:?}").into(),
                );
            }
            gate.release();
            drop(release);
            let action_result = action
                .join()
                .map_err(|_| "accepted action thread panicked")?;
            if let Ok(action_response) = action_result {
                let action: CommandResponse = response(action_response)?;
                if action.outcome != sts2_harness::management::CommandOutcome::Pending {
                    return Err(format!("accepted action unexpectedly settled: {action:?}").into());
                }
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let status = loop {
                let status: Value = response(client.request_json(
                    "GET",
                    &format!("/v1/workflow-runs/{}", submitted.run_id),
                    None,
                )?)?;
                if status["run"]["pending_operation"]["state"] == "accepted" {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    return Err(
                        format!("accepted operation was not durably retained: {status}").into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            gate.settle();
            let run_id = submitted.run_id;
            let revision = status["run"]["run_revision"]
                .as_u64()
                .ok_or("retained run omitted revision")?;
            let operation_id = status["run"]["pending_operation"]["operation_id"]
                .as_str()
                .ok_or("accepted barrier omitted its pending operation identity")?;
            if status["run"]["status"] != "needs_operator"
                || status["run"]["pending_operation"]["operation_id"] != operation_id
                || status["run"]["pending_operation"]["state"] != "accepted"
            {
                return Err(format!(
                    "accepted action was not retained for reconciliation: {status}"
                )
                .into());
            }
            let cancel = CommandRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                command_id: "served-cancel-after-accepted-barrier".to_owned(),
                run_id: run_id.clone(),
                expected_revision: revision,
                actor_scope: "profile:served".to_owned(),
                kind: CommandKind::Cancel,
                parameters: CommandParameters::default(),
            };
            let cancelled: CommandResponse = response(client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{run_id}/commands"),
                Some(&serde_json::to_vec(&cancel)?),
            )?)?;
            if cancelled.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err(format!("served cancellation was not applied: {cancelled:?}").into());
            }
            let final_status: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}"),
                None,
            )?)?;
            if final_status["run"]["status"] != "cancelled"
                || !final_status["run"]["pending_operation"].is_null()
            {
                return Err(format!(
                    "served cancellation did not dominate after reconciliation: {final_status}"
                )
                .into());
            }
            let dominated = CommandRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                command_id: "served-step-after-cancel".to_owned(),
                run_id: run_id.clone(),
                expected_revision: cancelled.run_revision,
                actor_scope: "profile:served".to_owned(),
                kind: CommandKind::Step,
                parameters: CommandParameters::default(),
            };
            let dominated: CommandResponse = response(client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{run_id}/commands"),
                Some(&serde_json::to_vec(&dominated)?),
            )?)?;
            if dominated.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err("post-cancel step was not dominated".into());
            }
            Ok(())
        })();
        let output = stop(service)?;
        attempt.map_err(|error| {
            format!(
                "served accepted-barrier cancellation failed: {error}; service_stdout={}; service_stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        })?;
        assert_killed(&output, "served accepted-barrier workflow")
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    result.map_err(|error| {
        gateway_failure_evidence(
            "cancellation",
            &format!("served accepted-barrier cancellation: {error}"),
            &gateway_output,
        )
    })?;
    if gateway_output.status.code() != Some(0) && !gateway_output.status.signal().is_some() {
        return Err(format!("gateway cleanup failed: {}", gateway_output.status).into());
    }
    if !ledger.errors.is_empty() {
        return Err(format!("served cancellation fixture failed: {:?}", ledger.errors).into());
    }
    let actions: Vec<_> = ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-action")
        .collect();
    let reconciles: Vec<_> = ledger
        .requests
        .iter()
        .filter(|request| request.path.starts_with("/api/v4/runtime/expert-actions/"))
        .collect();
    let operation_id = actions
        .first()
        .and_then(|request| request.body["operation_id"].as_str());
    if actions.len() != 1
        || reconciles.len() < 2
        || operation_id.is_none_or(|operation_id| {
            reconciles.iter().any(|request| {
                request.path != format!("/api/v4/runtime/expert-actions/{operation_id}")
            })
        })
    {
        return Err(format!(
            "accepted action did not retain one identity through barrier and reconciliation: {:?}",
            paths(&ledger)
        )
        .into());
    }
    if !ledger.responses.iter().any(|response| {
        response.body["status"] == "settled"
            && response.body["operation_id"] == operation_id.unwrap_or_default()
    }) {
        return Err("cancellation reconciliation lacked a settled operation response".into());
    }
    Ok(())
}
