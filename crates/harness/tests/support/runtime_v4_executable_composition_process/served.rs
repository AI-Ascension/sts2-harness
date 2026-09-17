// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::Connection;
use sts2_harness::{ExecutionStore, OperationState, StoredOperation};

#[path = "served/session.rs"]
mod session;
use session::{
    WorkflowServiceConfig, response, served_runtime_run_id, submit_and_step_policy_gate,
    wait_for_workflow_service, workflow_service_command,
};

#[path = "served/context_source.rs"]
mod context_source;
pub(crate) use context_source::run_served_context_source_adoption;

#[path = "served/cancellation.rs"]
mod cancellation;
pub(crate) use cancellation::run_served_cancel_after_accepted_barrier;

#[path = "served/receipt_recovery.rs"]
mod receipt_recovery;
pub(crate) use receipt_recovery::run_served_context_receipt_recovery;

type RestartScenarioResult = (Output, Option<Output>, Option<StoredOperation>);

pub(crate) fn run_served_policy_gate(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_served_policy_gate_inner(gateway_binary, mcp_binary, harness_binary, false)
}

pub(crate) fn run_served_restart_refuses_duplicate_effect(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_served_policy_gate_inner(gateway_binary, mcp_binary, harness_binary, true)
}

fn run_served_policy_gate_inner(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    restart_after_unknown: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge()?;
    let mod_server = ModServer::new(if restart_after_unknown {
        FixtureMode::UnknownOperation
    } else {
        FixtureMode::Success
    })?;
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
    };
    let mut gateway = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<RestartScenarioResult, Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway, gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let first_attempt = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            submit_and_step_policy_gate(&client, if restart_after_unknown { 3 } else { 4 })
        })();
        let first_output = stop(service)?;
        let submission = first_attempt.map_err(|error| {
            format!(
                "first served workflow attempt failed: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&first_output.stdout),
                String::from_utf8_lossy(&first_output.stderr),
            )
        })?;
        if !restart_after_unknown {
            return Ok((first_output, None, None));
        }

        let operation_id = submission
            .operation_id
            .as_deref()
            .ok_or("unknown served run omitted its operation identity")?;
        let operation = assert_durable_unknown(
            &workflow_store,
            &execution_store,
            &submission.run_id,
            operation_id,
        )?;

        let mut restarted_service = workflow_service_command(&service_config)?.spawn()?;
        let restart_attempt: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut restarted_service, workflow_address)?;
            let status_response = client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{}", submission.run_id),
                None,
            )?;
            let status: Value = response(status_response)?;
            if status["run"]["status"] != "needs_operator"
                || status["run"]["pending_operation"]["operation_id"] != operation_id
                || status["run"]["pending_operation"]["state"] != "unknown"
                || status["recovery_admission"]["kind"] != "reconciling"
            {
                return Err(
                    format!("restarted service lost unknown workflow state: {status}").into(),
                );
            }
            let command = CommandRequest {
                schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                command_id: "served-step-after-restart".to_owned(),
                run_id: submission.run_id.clone(),
                expected_revision: submission.revision,
                actor_scope: "profile:served".to_owned(),
                kind: CommandKind::Step,
                parameters: CommandParameters::default(),
            };
            let refused = client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{}/commands", submission.run_id),
                Some(&serde_json::to_vec(&command)?),
            )?;
            let body: Value = serde_json::from_slice(&refused.body)?;
            if refused.status != 409 || body["error"]["code"] != "live_runtime_after_restart" {
                return Err(format!(
                    "restarted service did not refuse the live step: HTTP {} {body}",
                    refused.status
                )
                .into());
            }
            Ok(())
        })();
        let restarted_output = stop(restarted_service)?;
        restart_attempt?;
        let operation_after_restart =
            ExecutionStore::open_read_only(&execution_store)?.operation(operation_id)?;
        if operation_after_restart != operation {
            return Err("restart refusal changed the durable unknown operation record".into());
        }
        Ok((first_output, Some(restarted_output), Some(operation)))
    })();
    let gateway_output = stop(gateway)?;
    let ledger = mod_server.finish();
    let (service_output, restarted_output, operation) = result?;
    assert_killed(&service_output, "first served workflow")?;
    if let Some(output) = &restarted_output {
        assert_killed(output, "restarted served workflow")?;
    }
    if gateway_output.status.code() != Some(0) && !gateway_output.status.signal().is_some() {
        return Err(format!("gateway cleanup failed: {}", gateway_output.status).into());
    }
    if !ledger.errors.is_empty() {
        return Err(format!("served fixture failed: {:?}", ledger.errors).into());
    }
    if !ledger
        .requests
        .iter()
        .any(|request| request.path == "/api/v4/runtime/expert-state")
    {
        return Err("served workflow did not reach the authoritative MCP observation".into());
    }
    let actions: Vec<_> = ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-action")
        .collect();
    let Some(action) = actions.first() else {
        return Err("adopted provider policy did not dispatch an action".into());
    };
    let Some(operation_id) = action.body["operation_id"].as_str() else {
        return Err("served action omitted its operation identity".into());
    };
    let settled: Vec<_> = ledger
        .responses
        .iter()
        .filter(|response| {
            response.body["status"] == "settled"
                && response.body["operation_id"] == operation_id
                && response.body["state_id"] == "live:8"
                && response.body["generation"] == 8
        })
        .collect();
    if actions.len() != 1
        || actions[0].body["state_id"] != "live:7"
        || actions[0].body["generation"] != 7
        || action.body["action"]["action_id"] != ACTION_ID
        || (operation.is_none()
            && (ledger
                .requests
                .iter()
                .filter(|request| {
                    request.path == format!("/api/v4/runtime/expert-actions/{operation_id}")
                })
                .count()
                != 1
                || settled.len() != 1))
        || (operation.is_some()
            && (!settled.is_empty()
                || !ledger.responses.iter().any(|response| {
                    response.body["status"] == "unknown"
                        && response.body["operation_id"] == operation_id
                })
                || operation.as_ref().is_some_and(|record| {
                    record.state != OperationState::Unknown
                        || record.intent.generation != 7
                        || record.intent.action_id != ACTION_ID
                })))
    {
        return Err(format!(
            "served provider policy did not preserve the expected fenced action outcome: {:?}",
            paths(&ledger)
        )
        .into());
    }
    Ok(())
}

fn assert_killed(output: &Output, process: &str) -> Result<(), Box<dyn std::error::Error>> {
    if output.status.code() != Some(0) && !output.status.signal().is_some_and(|signal| signal == 9)
    {
        return Err(format!(
            "{process} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn assert_durable_unknown(
    workflow_store: &Path,
    execution_store: &Path,
    workflow_run_id: &str,
    operation_id: &str,
) -> Result<StoredOperation, Box<dyn std::error::Error>> {
    let connection = Connection::open(workflow_store)?;
    let workflow_snapshot = connection
        .query_row(
            "SELECT snapshot FROM management_runs WHERE workflow_run_id = ?1",
            [workflow_run_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(|error| format!("read persisted workflow snapshot: {error}"))?;
    let snapshot: Value = serde_json::from_slice(&workflow_snapshot)?;
    let workflow_event_count = connection
        .query_row(
            "SELECT COUNT(*) FROM management_events WHERE workflow_run_id = ?1",
            [workflow_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("read persisted workflow events: {error}"))?;
    if snapshot["status"] != "needs_operator"
        || snapshot["pending_operation"]["operation_id"] != operation_id
        || snapshot["pending_operation"]["state"] != "unknown"
        || workflow_event_count <= 0
    {
        return Err(
            "workflow snapshot and event journal did not persist the unknown operation".into(),
        );
    }
    drop(connection);

    let operation = ExecutionStore::open_read_only(execution_store)?
        .operation(operation_id)
        .map_err(|error| format!("read persisted execution operation: {error}"))?;
    if operation.state != OperationState::Unknown
        || operation.intent.state_id != "live:7"
        || operation.intent.generation != 7
        || operation.intent.action_id != ACTION_ID
    {
        return Err("execution journal did not retain the original unknown operation".into());
    }
    Ok(operation)
}

fn policy_config(path: &Path, runtime_run_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let capabilities = NativeCapabilities::fixture();
    Ok(serde_json::to_string(&json!({
        "schema_version": "ascension.workflow-provider-policy-config.v1",
        "store_path": path,
        "key_reference": "STS2_SERVED_PROVIDER_POLICY_KEY",
        "scope": policy_scope(runtime_run_id)?,
        "capabilities": capabilities,
        "selected_profile": "codex-app-server-fixture-v1"
    }))?)
}

fn policy_scope(request_id: &str) -> Result<SessionScope, Box<dyn std::error::Error>> {
    SessionScope::new(
        "served-policy-project",
        request_id,
        "episode-served-policy-gate",
        "served-policy-agent",
    )
    .map_err(|error| format!("fixture provider-policy scope is invalid: {error}").into())
}

fn seed_adopted_runtime_policy(
    path: &Path,
    runtime_run_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let scope = policy_scope(runtime_run_id)?;
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.effective_limits.max_completed_turns = 1;
    capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
    let store = ProviderSessionMetadataStore::encrypted(path, [0x11; 32], scope.clone())
        .map_err(|error| format!("create provider-policy store: {error}"))?;
    let owner = ProviderSessionPolicyOwner::open(store, scope.clone(), capabilities.clone())
        .map_err(|error| format!("open provider-policy owner: {error}"))?;
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = sts2_harness::provider_session::ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "served-fixture-realm".to_owned();
    policy.max_completed_turns = 1;
    policy.profile_sha256 = capabilities.profile_sha256.clone();
    let sha256 = owner
        .import(serde_json::to_vec(&policy)?)
        .map_err(|error| format!("import provider policy: {error}"))?;
    owner
        .adopt_imported(&sha256, 2)
        .map_err(|error| format!("adopt imported provider policy: {error}"))?;
    Ok(())
}

pub(crate) fn paths(ledger: &DownstreamLedger) -> Vec<String> {
    ledger
        .requests
        .iter()
        .map(|request| request.path.clone())
        .collect()
}
