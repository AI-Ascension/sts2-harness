// SPDX-License-Identifier: MIT

use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_harness::management::{
    CommandKind, CommandParameters, CommandRequest, CommandResponse, MANAGEMENT_SCHEMA_VERSION,
    ManagementClient, RunRequest, RunTargetConfiguration, TARGET_ADMISSION_SCHEMA_VERSION,
    TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionRequest, TargetCatalogResponse,
    TargetPreflightResponse, digest_value,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionPolicy,
    ProviderSessionPolicyOwner, SessionScope,
};

use super::fixture::{
    ACTION_ID, CALLER_ID, DownstreamLedger, FixtureMode, INSTANCE_ID, LEASE_EPOCH, LEASE_ID,
    MCP_SESSION_ID, ModServer, REVIEWED_EXO_REVISION, SESSION_ID,
};

pub(crate) struct ScenarioResult {
    runtime: Output,
    gateway: Output,
    ledger: DownstreamLedger,
}

pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-runtime-v4-executable-composition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self { path })
    }

    pub(crate) fn bridge(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = self.path.join("bounded-exo-bridge.sh");
        fs::write(
            &path,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"decision\":\"action\",\"action_id\":\"potion:7:potion:fire:enemy:1\",\"rationale\":\"use the visible potion\"}'\n",
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn executable(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required for this ignored operator test"))?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{name} is not a file").into())
    }
}

fn free_address() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?)
}

fn gateway(
    binary: &Path,
    address: SocketAddr,
    mod_address: SocketAddr,
) -> Result<Child, Box<dyn std::error::Error>> {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_MOD_ADDR", mod_address.to_string())
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MOD_TOKEN", "mod-token")
        .env("STS2_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", LEASE_ID)
        .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command.spawn()?)
}

fn ready(child: &mut Child, address: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(format!("gateway exited: {status}").into());
        }
        if TcpStream::connect(address).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("gateway readiness deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn stop(mut child: Child) -> Result<Output, Box<dyn std::error::Error>> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    Ok(child.wait_with_output()?)
}

fn bounded(mut command: Command) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("runtime deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn run_scenario(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    bridge: &Path,
    mode: FixtureMode,
) -> Result<ScenarioResult, Box<dyn std::error::Error>> {
    let mod_server = ModServer::new(mode)?;
    let address = free_address()?;
    let execution_store = bridge
        .parent()
        .map(|path| {
            path.join(match mode {
                FixtureMode::Success => "execution-success.sqlite3",
                FixtureMode::ForeignExpertState => "execution-foreign.sqlite3",
                FixtureMode::MalformedExpertState => "execution-malformed.sqlite3",
            })
        })
        .ok_or("synthetic bridge has no parent directory")?;
    let mut gateway_process = gateway(gateway_binary, address, mod_server.address)?;
    let runtime = (|| {
        ready(&mut gateway_process, address)?;
        let mut command = Command::new(harness_binary);
        command
            .env_clear()
            .env("STS2_EXECUTION_STORE_PATH", execution_store)
            .env("STS2_GATEWAY_ADDR", address.to_string())
            .env("STS2_GATEWAY_TOKEN", "gateway-token")
            .env("STS2_MCP_BINARY", mcp_binary)
            .env("STS2_RUNTIME_PROFILE", "runtime-v4-expert")
            .env("STS2_INSTANCE_ID", INSTANCE_ID)
            .env("STS2_CALLER_ID", CALLER_ID)
            .env("STS2_SESSION_ID", SESSION_ID)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
            .env("STS2_LEASE_ID", LEASE_ID)
            .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
            .env("STS2_RUN_ID", "run-executable-composition")
            .env("STS2_EPISODE_ID", "episode-executable-composition")
            .env("STS2_TRAJECTORY_ID", "trajectory-executable-composition")
            .env("STS2_TRACE_ID", "trace-executable-composition")
            .env("STS2_ARTIFACT_ID", "artifact-executable-composition")
            .env("STS2_EXO_REVISION", REVIEWED_EXO_REVISION)
            .env("STS2_PROVIDER_KIND", "synthetic")
            // The synthetic probe is a raw-wire bridge: acknowledge it explicitly instead of
            // claiming the reviewed one-turn envelope admission.
            .env("STS2_EXO_ADMISSION", "legacy")
            .env("STS2_EXO_BRIDGE_BINARY", bridge)
            .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
            .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
            .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
            .env("STS2_OBJECTIVE", "exercise served policy gate")
            .env("STS2_MAX_STEPS", "4")
            .env("STS2_BARRIER_MAX_POLLS", "1")
            .env("STS2_BARRIER_WAIT_MILLIS", "1")
            .env("STS2_RECOVERY_MAX_ATTEMPTS", "2")
            .env("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS", "0")
            .env("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS", "1")
            .env(
                "STS2_OBJECTIVE",
                "reach the bounded synthetic terminal state",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        bounded(command)
    })();
    let gateway_output = stop(gateway_process)?;
    Ok(ScenarioResult {
        runtime: runtime?,
        gateway: gateway_output,
        ledger: mod_server.finish(),
    })
}

/// Starts the actual `serve-workflow` binary against gateway/MCP processes.
/// The provider-policy store and runtime configuration are both bound to the
/// deterministic management run identity before the service accepts a step.
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
        "../../../../conformance/workflow-v1/valid-strict.json"
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

fn paths(ledger: &DownstreamLedger) -> Vec<String> {
    ledger
        .requests
        .iter()
        .map(|request| request.path.clone())
        .collect()
}

pub(crate) fn assert_success(
    result: &ScenarioResult,
) -> Result<String, Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(0) {
        return Err(format!(
            "runtime failed: {}",
            String::from_utf8_lossy(&result.runtime.stderr)
        )
        .into());
    }
    if !result.ledger.errors.is_empty() {
        return Err(format!("fixture failed: {:?}", result.ledger.errors).into());
    }
    let action = result
        .ledger
        .requests
        .iter()
        .find(|request| request.path == "/api/v4/runtime/expert-action")
        .ok_or("expert action missing")?;
    let operation = action.body["operation_id"]
        .as_str()
        .ok_or("operation missing")?
        .to_owned();
    let expected = [
        "/api/v3/runtime/state",
        "/api/v4/runtime/expert-state",
        "/api/v3/runtime/legal-actions",
        "/api/v4/runtime/expert-state",
        "/api/v4/runtime/expert-action",
    ];
    let actual = paths(&result.ledger);
    if actual.len() != 6
        || actual[..5] != expected
        || actual[5] != format!("/api/v4/runtime/expert-actions/{operation}")
    {
        return Err(format!("unexpected path ledger: {actual:?}").into());
    }
    let methods: Vec<&str> = result
        .ledger
        .requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    if methods != ["GET", "GET", "GET", "GET", "POST", "GET"] {
        return Err(format!("unexpected downstream methods: {methods:?}").into());
    }
    let statuses: Vec<u16> = result
        .ledger
        .responses
        .iter()
        .map(|response| response.status)
        .collect();
    if statuses != [200, 200, 200, 200, 503, 200] {
        return Err(format!("unexpected downstream response statuses: {statuses:?}").into());
    }
    if action.body["state_id"] != "live:7"
        || action.body["generation"] != 7
        || action.body["action"]["action_id"] != ACTION_ID
        || action.body["action"]["action"]["kind"] != "use_potion"
        || action.body["status"] != Value::Null
    {
        return Err("action fence mismatch".into());
    }
    let reconcile = &result.ledger.requests[5];
    if reconcile.body != Value::Null
        || reconcile.headers.get("x-sts2-lease-id").map(String::as_str) != Some(LEASE_ID)
        || reconcile
            .headers
            .get("x-sts2-lease-epoch")
            .map(String::as_str)
            != Some("1")
    {
        return Err("reconcile lease mismatch".into());
    }
    let unknown = &result.ledger.responses[4].body;
    if unknown["status"] != "unknown"
        || unknown["operation_id"] != operation
        || unknown["state_id"] != "live:7"
        || unknown["generation"] != 7
    {
        return Err("unknown response identity mismatch".into());
    }
    let settled = &result.ledger.responses[5].body;
    if settled["status"] != "settled"
        || settled["operation_id"] != operation
        || settled["state_id"] != "live:8"
        || settled["generation"] != 8
        || settled["observation"]["state_id"] != "live:8"
        || settled["observation"]["generation"] != 8
    {
        return Err("settled response identity mismatch".into());
    }
    let report: Value = serde_json::from_slice(&result.runtime.stdout)
        .map_err(|error| format!("runtime report is not JSON: {error}"))?;
    if report["protocol"] != "runtime-v4-expert"
        || report["status"] != "complete"
        || report["terminal_stage"] != "victory"
        || report["final_generation"] != 8
        || report["transitions"] != 1
    {
        return Err(format!("runtime completion report mismatch: {report}").into());
    }
    Ok(operation)
}

pub(crate) fn assert_foreign_state_rejected(
    result: &ScenarioResult,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(2) {
        return Err(format!("foreign state exit: {:?}", result.runtime.status.code()).into());
    }
    let methods: Vec<&str> = result
        .ledger
        .requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    if !result.ledger.errors.is_empty()
        || paths(&result.ledger) != ["/api/v3/runtime/state", "/api/v4/runtime/expert-state"]
        || methods != ["GET", "GET"]
        || result.ledger.responses.len() != 2
        || result.ledger.responses[0].status != 200
        || result.ledger.responses[1].status != 200
        || result.ledger.responses[1].body["state_id"] != "foreign-state"
    {
        return Err(format!(
            "foreign state was not rejected at composition: exit={:?}, errors={:?}, paths={:?}, methods={methods:?}, responses={:?}, stderr={}",
            result.runtime.status.code(),
            result.ledger.errors,
            paths(&result.ledger),
            result.ledger.responses,
            String::from_utf8_lossy(&result.runtime.stderr),
        )
        .into());
    }
    Ok(())
}

include!("runtime_v4_executable_composition_malformed.rs");

pub(crate) fn write_evidence(
    success: &ScenarioResult,
    foreign: &ScenarioResult,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("runtime-success.stdout"), &success.runtime.stdout)?;
    fs::write(root.join("runtime-success.stderr"), &success.runtime.stderr)?;
    fs::write(root.join("gateway-success.stdout"), &success.gateway.stdout)?;
    fs::write(root.join("gateway-success.stderr"), &success.gateway.stderr)?;
    fs::write(root.join("runtime-foreign.stdout"), &foreign.runtime.stdout)?;
    fs::write(root.join("runtime-foreign.stderr"), &foreign.runtime.stderr)?;
    fs::write(root.join("gateway-foreign.stdout"), &foreign.gateway.stdout)?;
    fs::write(root.join("gateway-foreign.stderr"), &foreign.gateway.stderr)?;
    let summary = |result: &ScenarioResult| {
        Value::Array(
            result
                .ledger
                .requests
                .iter()
                .zip(&result.ledger.responses)
                .map(|(request, response)| {
                    json!({"method":request.method,"path":request.path,"operation_id":request.body["operation_id"],"state_id":request.body["state_id"],"generation":request.body["generation"],"status":request.body["status"],"response_status":response.status,"response_operation_id":response.body["operation_id"],"response_state_id":response.body["state_id"],"response_generation":response.body["generation"],"response_status_value":response.body["status"]})
                })
                .collect(),
        )
    };
    fs::write(
        root.join("downstream-success.json"),
        serde_json::to_vec_pretty(&summary(success))?,
    )?;
    fs::write(
        root.join("downstream-foreign.json"),
        serde_json::to_vec_pretty(&summary(foreign))?,
    )?;
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"status":"confirmed","scope":"source-derived executable composition","operation_id":operation,"success_exit":success.runtime.status.code(),"foreign_exit":foreign.runtime.status.code(),"provider":"synthetic bounded bridge","game":"synthetic downstream"}),
        )?,
    )?;
    Ok(())
}
