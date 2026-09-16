// SPDX-License-Identifier: MIT

use super::*;

pub(crate) fn run_served_policy_gate(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let bridge = temporary.bridge()?;
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let policy_store = temporary.path.join("served-provider-policy.sqlite3");
    let context_store = temporary.path.join("served-context.sqlite3");
    let execution_store = temporary.path.join("served-execution.sqlite3");
    let runtime_run_id = served_runtime_run_id()?;
    seed_adopted_runtime_policy(&policy_store, &runtime_run_id)?;
    let mut gateway = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<Output, Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway, gateway_address)?;
        let mut command = Command::new(harness_binary);
        command
            .arg("serve-workflow")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_WORKFLOW_LISTEN", workflow_address.to_string())
            .env(
                "STS2_WORKFLOW_STORE",
                temporary.path.join("workflow.sqlite3"),
            )
            .env("STS2_WORKFLOW_AUTH_PROFILE", "served")
            .env("STS2_WORKFLOW_TOKEN_SERVED", "served-workflow-token")
            .env(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG",
                policy_config(&policy_store, &runtime_run_id)?,
            )
            .env(
                "STS2_SERVED_PROVIDER_POLICY_KEY",
                "1111111111111111111111111111111111111111111111111111111111111111",
            )
            .env(
                "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG",
                serde_json::to_string(&json!({
                    "schema_version":"ascension.workflow-context-owner-config.v1",
                    "store_path":context_store,
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
            .env("STS2_EXECUTION_STORE_PATH", execution_store)
            .env("STS2_GATEWAY_ADDR", gateway_address.to_string())
            .env("STS2_GATEWAY_TOKEN", "gateway-token")
            .env("STS2_MCP_BINARY", mcp_binary)
            .env("STS2_RUNTIME_PROFILE", "runtime-v4-expert")
            .env("STS2_INSTANCE_ID", INSTANCE_ID)
            .env("STS2_CALLER_ID", CALLER_ID)
            .env("STS2_SESSION_ID", SESSION_ID)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
            .env("STS2_LEASE_ID", LEASE_ID)
            .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
            .env("STS2_RUN_ID", &runtime_run_id)
            .env("STS2_EPISODE_ID", "episode-served-policy-gate")
            .env("STS2_TRAJECTORY_ID", "trajectory-served-policy-gate")
            .env("STS2_TRACE_ID", "trace-served-policy-gate")
            .env("STS2_ARTIFACT_ID", "artifact-served-policy-gate")
            .env("STS2_EXO_REVISION", REVIEWED_EXO_REVISION)
            // The bridge is a test-only raw-wire provider and is intentionally
            // not a reviewed or paid/native provider.
            .env("STS2_EXO_ADMISSION", "legacy")
            .env("STS2_EXO_BRIDGE_BINARY", bridge)
            .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
            .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
            .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
            .env("STS2_OBJECTIVE", "exercise served policy gate")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut service = command.spawn()?;
        let client = wait_for_workflow_service(&mut service, workflow_address)?;
        let submission = submit_and_step_policy_gate(&client);
        let output = stop(service)?;
        submission?;
        Ok(output)
    })();
    let gateway_output = stop(gateway)?;
    let ledger = mod_server.finish();
    let service_output = result?;
    if service_output.status.code() != Some(0)
        && !service_output
            .status
            .signal()
            .is_some_and(|signal| signal == 9)
    {
        return Err(format!(
            "served workflow failed: {}",
            String::from_utf8_lossy(&service_output.stderr)
        )
        .into());
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
        || ledger
            .requests
            .iter()
            .filter(|request| {
                request.path == format!("/api/v4/runtime/expert-actions/{operation_id}")
            })
            .count()
            != 1
        || settled.len() != 1
    {
        return Err(format!(
            "adopted provider policy did not dispatch and settle one fenced action: {:?}",
            paths(&ledger)
        )
        .into());
    }
    Ok(())
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

fn wait_for_workflow_service(
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

fn submit_and_step_policy_gate(
    client: &ManagementClient,
) -> Result<(), Box<dyn std::error::Error>> {
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
    let mut revision = snapshot["run_revision"]
        .as_u64()
        .ok_or("served submission omitted run revision")?;
    for index in 1..=4 {
        let command = CommandRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            command_id: format!("served-step-{index}"),
            run_id: run_id.to_owned(),
            expected_revision: revision,
            actor_scope: "profile:served".to_owned(),
            kind: CommandKind::Step,
            parameters: CommandParameters::default(),
        };
        let command = response::<CommandResponse>(client.request_json(
            "POST",
            &format!("/v1/workflow-runs/{run_id}/commands"),
            Some(&serde_json::to_vec(&command)?),
        )?)?;
        if !matches!(
            command.outcome,
            sts2_harness::management::CommandOutcome::Applied
                | sts2_harness::management::CommandOutcome::Pending
        ) {
            return Err(
                format!("served step {index} was not applied: {:?}", command.outcome).into(),
            );
        }
        revision = command.run_revision;
    }
    Ok(())
}

fn served_runtime_run_id() -> Result<String, Box<dyn std::error::Error>> {
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

fn served_definition() -> Result<Value, Box<dyn std::error::Error>> {
    let mut definition: Value = serde_json::from_slice(include_bytes!(
        "../../../../../conformance/workflow-v1/valid-strict.json"
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

fn response<T: serde::de::DeserializeOwned>(
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

pub(crate) fn paths(ledger: &DownstreamLedger) -> Vec<String> {
    ledger
        .requests
        .iter()
        .map(|request| request.path.clone())
        .collect()
}
