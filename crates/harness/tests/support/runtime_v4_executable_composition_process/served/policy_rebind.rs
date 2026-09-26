// SPDX-License-Identifier: MIT

use super::*;
use session::{
    WorkflowServiceConfig, response, served_runtime_run_id, submit_policy_gate,
    wait_for_workflow_service, workflow_service_command,
};
use sts2_harness::management::{
    CommandKind, CommandParameters, CommandRequest, CommandResponse, MANAGEMENT_SCHEMA_VERSION,
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, ProviderSessionPolicyAdoptImportedRequest,
    ProviderSessionPolicyCommandResponse, ProviderSessionPolicyViewResponse,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionMode, ProviderSessionPolicy, SessionScope,
};

/// Drives the served live path through an idle (between-node) adoption of a
/// **changed** provider-session policy and then issues the decision that follows
/// it. Before #255 this failed permanently with `provider_session_policy_changed`
/// because the session had pinned the launch-time binding and no supported path
/// could open the fresh session the error demanded.
///
/// The changed policy is adopted through the production HTTP routes, so the
/// owner generation really moves. Harness/Gateway/MCP are real peer processes;
/// the downstream mod endpoint and provider bridge are synthetic fixtures.
pub(crate) fn run_served_policy_rebind_after_idle_adoption(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let provider_capture = temporary.path.join("provider-request.json");
    let bridge = temporary.bridge_capturing(&provider_capture)?;
    let mod_server = ModServer::new(FixtureMode::Success)?;
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
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let attempt: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submission = submit_policy_gate(&client)?;
            let run_id = submission.run_id.as_str();

            let observed = step(&client, run_id, submission.revision, "rebind-observe")?;
            if observed.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err("served observation step was not applied".into());
            }
            let after_observe = run_snapshot(&client, run_id)?;
            if after_observe["status"] != "running"
                || after_observe["cursor"]["node_id"] != "decide"
                || after_observe["budget"]["provider_calls_consumed"] != 0
            {
                return Err(format!(
                    "served run did not reach an idle decide cursor: {after_observe}"
                )
                .into());
            }

            // Adopt a *changed* policy while the run is idle. This is the exact
            // sequence the issue documents; nothing has been inferred yet, so the
            // next decision must pick the binding up instead of fencing the run.
            let path = format!("/v1/workflow-runs/{run_id}/provider-session-policy");
            let before: ProviderSessionPolicyViewResponse =
                response(client.request_json("GET", &path, None)?)?;
            let previous_sha256 = before
                .value
                .active
                .as_ref()
                .ok_or("served policy owner has no active policy")?
                .sha256
                .clone();
            let changed = changed_policy(&runtime_run_id)?;
            let changed_sha256 = sts2_harness::sha256_hex(&changed);
            let imported: ProviderSessionPolicyCommandResponse = response(client.request_json(
                "POST",
                &format!("{path}/import?expected_revision={}", before.value.revision),
                Some(&changed),
            )?)?;
            if imported.operation != "import"
                || imported.policy_sha256.as_deref() != Some(changed_sha256.as_str())
                || imported.inference_calls != 0
                || imported.game_effects != 0
            {
                return Err(format!(
                    "served policy import was not a metadata-only change: {imported:?}"
                )
                .into());
            }
            let adopt = ProviderSessionPolicyAdoptImportedRequest {
                schema_version: PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION.to_owned(),
                policy_sha256: changed_sha256.clone(),
            };
            let adopted: ProviderSessionPolicyCommandResponse = response(client.request_json(
                "POST",
                &format!("{path}/adoptions?expected_revision={}", imported.revision),
                Some(&serde_json::to_vec(&adopt)?),
            )?)?;
            if adopted.operation != "adopt"
                || adopted.policy_sha256.as_deref() != Some(changed_sha256.as_str())
                || adopted.inference_calls != 0
                || adopted.game_effects != 0
            {
                return Err(format!(
                    "served policy adoption was not a metadata-only change: {adopted:?}"
                )
                .into());
            }
            if changed_sha256 == previous_sha256 {
                return Err("policy-rebind fixture did not change the active policy".into());
            }

            // The decision after the idle adoption must execute, not fence.
            let decided = step(&client, run_id, observed.run_revision, "rebind-decide")?;
            if decided.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err(
                    format!("served decision after adoption was not applied: {decided:?}").into(),
                );
            }
            let after_decision = run_snapshot(&client, run_id)?;
            if after_decision["status"] != "running"
                || after_decision["cursor"]["node_id"] != "execute"
                || after_decision["budget"]["provider_calls_consumed"] != 1
            {
                let events: Value = response(client.request_json(
                    "GET",
                    &format!("/v1/workflow-runs/{run_id}/events?after_sequence=0&limit=128"),
                    None,
                )?)?;
                return Err(format!(
                    "served decision did not survive the idle policy adoption: run={after_decision}; events={events}"
                )
                .into());
            }
            Ok(())
        })();
        let output = stop(service)?;
        attempt.map_err(|error| {
            format!(
                "served policy-rebind attempt failed: {error}; service_stdout={}; service_stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        })?;
        assert_killed(&output, "served policy-rebind workflow")?;
        Ok(())
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    if gateway_output.status.code() != Some(0) && gateway_output.status.signal().is_none() {
        return Err(format!("gateway cleanup failed: {}", gateway_output.status).into());
    }
    result.map_err(|error| {
        gateway_failure_evidence(&format!("served policy-rebind: {error}"), &gateway_output)
    })?;
    if !ledger.errors.is_empty() {
        return Err(format!("policy-rebind gateway fixture failed: {:?}", ledger.errors).into());
    }
    if !ledger
        .requests
        .iter()
        .any(|request| request.path == "/api/v4/runtime/expert-state")
    {
        return Err("policy-rebind run never reached observation".into());
    }
    // The rebind must not have skipped or duplicated the provider exchange, and
    // it must not have crossed the game boundary early.
    let exchange_count = fs::read(provider_capture.with_extension("count"))?;
    if exchange_count != b"x" {
        return Err(format!(
            "rebound decision did not perform exactly one provider exchange: {exchange_count:?}"
        )
        .into());
    }
    let actions: Vec<_> = ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-action")
        .collect();
    if !actions.is_empty() {
        return Err(format!(
            "policy-rebind fixture crossed the game boundary before execute: {:?}",
            paths(&ledger)
        )
        .into());
    }
    Ok(())
}

/// A schema-valid, executable policy that differs from the seeded one only in
/// `epoch`, so the owner records a genuine active-SHA change.
fn changed_policy(runtime_run_id: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let scope = SessionScope::new(
        "served-policy-project",
        runtime_run_id,
        "episode-served-policy-gate",
        "served-policy-agent",
    )?;
    let capabilities = NativeCapabilities::fixture();
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "served-fixture-realm".to_owned();
    policy.max_completed_turns = 1;
    policy.profile_sha256 = capabilities.profile_sha256.clone();
    policy.epoch = 2;
    policy.validate_schema()?;
    policy
        .admit_for_profile(&capabilities)
        .map_err(|error| format!("changed policy is not executable: {error:?}"))?;
    Ok(serde_json::to_vec(&policy)?)
}

fn run_snapshot(
    client: &ManagementClient,
    run_id: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let value: Value =
        response(client.request_json("GET", &format!("/v1/workflow-runs/{run_id}"), None)?)?;
    Ok(value["run"].clone())
}

fn step(
    client: &ManagementClient,
    run_id: &str,
    revision: u64,
    command_id: &str,
) -> Result<CommandResponse, Box<dyn std::error::Error>> {
    let command = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: command_id.to_owned(),
        run_id: run_id.to_owned(),
        expected_revision: revision,
        actor_scope: "profile:served".to_owned(),
        kind: CommandKind::Step,
        parameters: CommandParameters::default(),
    };
    response(client.request_json(
        "POST",
        &format!("/v1/workflow-runs/{run_id}/commands"),
        Some(&serde_json::to_vec(&command)?),
    )?)
}
