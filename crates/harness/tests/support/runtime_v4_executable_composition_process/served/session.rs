// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::process::CommandExt;

pub(super) struct WorkflowServiceConfig<'a> {
    pub(super) harness_binary: &'a Path,
    pub(super) mcp_binary: &'a Path,
    pub(super) bridge: &'a Path,
    pub(super) gateway_address: SocketAddr,
    pub(super) workflow_address: SocketAddr,
    pub(super) policy_store: &'a Path,
    pub(super) context_store: &'a Path,
    pub(super) execution_store: &'a Path,
    pub(super) workflow_store: &'a Path,
    pub(super) runtime_run_id: &'a str,
}

pub(super) fn workflow_service_command(
    config: &WorkflowServiceConfig<'_>,
) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::new(config.harness_binary);
    command
        .arg("serve-workflow")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_WORKFLOW_LISTEN", config.workflow_address.to_string())
        .env("STS2_WORKFLOW_STORE", config.workflow_store)
        .env("STS2_WORKFLOW_AUTH_PROFILE", "served")
        .env("STS2_WORKFLOW_TOKEN_SERVED", "served-workflow-token")
        .env(
            "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG",
            super::policy_config(config.policy_store, config.runtime_run_id)?,
        )
        .env(
            "STS2_SERVED_PROVIDER_POLICY_KEY",
            "1111111111111111111111111111111111111111111111111111111111111111",
        )
        .env(
            "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG",
            serde_json::to_string(&json!({
                "schema_version":"ascension.workflow-context-owner-config.v1",
                "store_path":config.context_store,
                "key_reference":"STS2_SERVED_CONTEXT_OWNER_KEY",
                "owner_id":"served-context-owner",
                "owner_version":"v1",
                "context_ref":"context.live.v1",
                "limits":{"max_items":64,"max_notes":16,"max_context_bytes":131072,"max_objective_bytes":512,"max_control_events":64}
            }))?,
        )
        .env(
            "STS2_SERVED_CONTEXT_OWNER_KEY",
            "2222222222222222222222222222222222222222222222222222222222222222",
        )
        .env("STS2_EXECUTION_STORE_PATH", config.execution_store)
        .env("STS2_GATEWAY_ADDR", config.gateway_address.to_string())
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MCP_BINARY", config.mcp_binary)
        .env("STS2_RUNTIME_PROFILE", "runtime-v4-expert")
        .env("STS2_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", LEASE_ID)
        .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
        .env("STS2_RUN_ID", config.runtime_run_id)
        .env("STS2_EPISODE_ID", "episode-served-policy-gate")
        .env("STS2_TRAJECTORY_ID", "trajectory-served-policy-gate")
        .env("STS2_TRACE_ID", "trace-served-policy-gate")
        .env("STS2_ARTIFACT_ID", "artifact-served-policy-gate")
        .env("STS2_EXO_REVISION", REVIEWED_EXO_REVISION)
        // This bounded raw-wire bridge is test-only and does not call a native provider.
        .env("STS2_EXO_ADMISSION", "legacy")
        .env("STS2_EXO_BRIDGE_BINARY", config.bridge)
        .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
        .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
        .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
        .env("STS2_OBJECTIVE", "exercise served policy gate")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    Ok(command)
}

pub(super) struct SubmittedRun {
    pub(super) run_id: String,
    pub(super) revision: u64,
    pub(super) operation_id: Option<String>,
}

pub(super) fn wait_for_workflow_service(
    service: &mut Child,
    address: SocketAddr,
) -> Result<ManagementClient, Box<dyn std::error::Error>> {
    let client = ManagementClient::new(address, "served-workflow-token")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = service.try_wait()? {
            return Err(format!("served workflow exited: {status}").into());
        }
        if client.request_json("GET", "/v1/health", None).is_ok() {
            return Ok(client);
        }
        if Instant::now() >= deadline {
            return Err("served workflow readiness deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn submit_and_step_policy_gate(
    client: &ManagementClient,
    steps: u64,
) -> Result<SubmittedRun, Box<dyn std::error::Error>> {
    let definition = served_definition()?;
    let request_id = "served-policy-gate";
    let digest = digest_value(&definition)?;
    let catalog = response::<TargetCatalogResponse>(client.request_json(
        "GET",
        "/v1/workflow-targets",
        None,
    )?)?;
    if catalog.schema_version != TARGET_CATALOG_SCHEMA_VERSION {
        return Err("served workflow returned an invalid target catalog".into());
    }
    let target = catalog
        .targets
        .into_iter()
        .next()
        .ok_or("served target is absent")?;
    let admission_request = TargetAdmissionRequest {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        workflow_definition_digest: digest,
        target: RunTargetConfiguration {
            instance_id: target.instance_id,
            execution_profile: "live.workflow.v1".to_owned(),
            execution_mode: sts2_harness::management::ExecutionMode::Live,
            workflow_revision: "0.1.0".to_owned(),
            compatibility_revision: target.compatibility_revision,
            capability_revision: target.capability_revision,
            game_profile: "sts2-live-v1".to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
    };
    let preflight = response::<TargetPreflightResponse>(client.request_json(
        "POST",
        "/v1/workflow-targets/preflight",
        Some(&serde_json::to_vec(&admission_request)?),
    )?)?;
    let run = RunRequest {
        schema_version: sts2_harness::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: INSTANCE_ID.to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(preflight.admission),
    };
    let submitted = client.request_json(
        "POST",
        "/v1/workflow-runs",
        Some(&serde_json::to_vec(&run)?),
    )?;
    if submitted.status != 200 {
        return Err(format!(
            "served policy gate did not accept submission: {}",
            String::from_utf8_lossy(&submitted.body)
        )
        .into());
    }
    let snapshot: Value = serde_json::from_slice(&submitted.body)?;
    let run_id = snapshot["workflow_run_id"]
        .as_str()
        .ok_or("served submission omitted workflow run identity")?;
    assert_served_policy_routes(client, run_id)?;
    let mut revision = snapshot["run_revision"]
        .as_u64()
        .ok_or("served submission omitted run revision")?;
    for index in 1..=steps {
        let command = CommandRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            command_id: format!("served-step-{index}"),
            run_id: run_id.to_owned(),
            expected_revision: revision,
            actor_scope: "profile:served".to_owned(),
            kind: CommandKind::Step,
            parameters: CommandParameters::default(),
        };
        let command = response::<CommandResponse>(
            client
                .request_json(
                    "POST",
                    &format!("/v1/workflow-runs/{run_id}/commands"),
                    Some(&serde_json::to_vec(&command)?),
                )
                .map_err(|error| format!("served step {index} request failed: {error}"))?,
        )?;
        if !matches!(
            command.outcome,
            sts2_harness::management::CommandOutcome::Applied
                | sts2_harness::management::CommandOutcome::Pending
        ) {
            return Err(
                format!("served step {index} was not applied: {:?}", command.outcome).into(),
            );
        }
        if steps == 3
            && index == steps
            && command.outcome != sts2_harness::management::CommandOutcome::Pending
        {
            return Err(format!(
                "served step {index} did not preserve the unknown operation: {:?}",
                command.outcome
            )
            .into());
        }
        if index == 2 {
            let binding: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}/executions/live.node.2/context-binding"),
                None,
            )?)?;
            if binding["binding"]["workflow_run_id"] != run_id
                || binding["binding"]["boundary"]["state_id"] != "live:7"
                || binding["binding"]["boundary"]["generation"] != 7
                || binding["binding"]["boundary"]["catalog_sha256"] == "catalog-unavailable"
            {
                return Err(
                    "served context binding does not retain the launch observation/catalog".into(),
                );
            }
        }
        revision = command.run_revision;
    }
    let status: Value =
        response(client.request_json("GET", &format!("/v1/workflow-runs/{run_id}"), None)?)?;
    let operation_id = status["run"]["pending_operation"]["operation_id"]
        .as_str()
        .map(str::to_owned);
    if steps == 3
        && (status["run"]["status"] != "needs_operator"
            || status["run"]["pending_operation"]["state"] != "unknown"
            || operation_id.is_none())
    {
        return Err(format!("served workflow did not retain Unknown at step 3: {status}").into());
    }
    Ok(SubmittedRun {
        run_id: run_id.to_owned(),
        revision,
        operation_id,
    })
}

fn assert_served_policy_routes(
    client: &ManagementClient,
    run_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = format!("/v1/workflow-runs/{run_id}/provider-session-policy");
    let policy: ProviderSessionPolicyViewResponse =
        response(client.request_json("GET", &path, None)?)?;
    let active = policy
        .value
        .active
        .ok_or("served policy owner has no active policy")?;
    if policy.value.run_id != run_id
        || policy.operation != "current"
        || policy.effect_class != "local_metadata_only"
        || policy.inference_calls != 0
        || policy.game_effects != 0
    {
        return Err("served policy GET returned an invalid run-scoped view".into());
    }
    let command = ProviderSessionPolicyAdoptImportedRequest {
        schema_version: PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION.to_owned(),
        policy_sha256: active.sha256.clone(),
    };
    let adopted: ProviderSessionPolicyCommandResponse = response(client.request_json(
        "POST",
        &format!(
            "{path}/adoptions?expected_revision={}",
            policy.value.revision
        ),
        Some(&serde_json::to_vec(&command)?),
    )?)?;
    if adopted.operation != "adopt"
        || adopted.policy_sha256.as_deref() != Some(active.sha256.as_str())
        || adopted.effect_class != "local_metadata_only"
        || adopted.inference_calls != 0
        || adopted.game_effects != 0
    {
        return Err("served policy command did not preserve the active run-scoped binding".into());
    }
    Ok(())
}

pub(super) fn served_runtime_run_id() -> Result<String, Box<dyn std::error::Error>> {
    let definition = served_definition()?;
    let digest = digest_value(&definition)?;
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "served-policy-gate".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: INSTANCE_ID.to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: None,
    };
    Ok(sts2_harness::management::live_run_id(&request, &digest)?)
}

pub(super) fn served_definition() -> Result<Value, Box<dyn std::error::Error>> {
    let mut definition: Value = serde_json::from_slice(include_bytes!(
        "../../../../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    definition["annotations"]["synthetic"] = json!(false);
    definition["game_profile"] = json!("sts2-live-v1");
    definition["policy_ref"] = json!("policy.live.v1");
    definition["graphs"][0]["nodes"][0]["config"]["projection_ref"] = json!("fair-play.live.v1");
    definition["graphs"][0]["nodes"][1]["config"]["decision_profile_ref"] =
        json!("decision.live.v1");
    definition["graphs"][0]["nodes"][1]["config"]["context_ref"] = json!("context.live.v1");
    definition["capabilities"]["required"][0] = json!("observe.fair-play.v1");
    Ok(definition)
}

pub(super) fn response<T: serde::de::DeserializeOwned>(
    response: sts2_harness::management::ClientResponse,
) -> Result<T, Box<dyn std::error::Error>> {
    if response.status != 200 {
        return Err(format!(
            "unexpected HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        )
        .into());
    }
    Ok(serde_json::from_slice(&response.body)?)
}
