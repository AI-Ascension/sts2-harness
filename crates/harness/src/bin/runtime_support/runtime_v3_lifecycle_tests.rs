// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use sts2_harness::exo_admission::{ExoAdmissionPlan, ExoRuntimeAdmission};
use sts2_harness::provider_session::{
    ProviderSessionMetadataStore, ProviderSessionMode, ProviderSessionPolicy,
    ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::{
    EXO_SOURCE_REVISION, ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExoConfig,
    ExoContextMode, ExoLimits, ExoPlatform, ExoProcessConfig, ExoProfile, ExoRestrictedProfile,
    ExoRuntime, ExoTrustedConfiguration, sha256_hex,
};

use super::super::{RuntimeV3Port, allocation_context, parse};
use super::*;
use crate::runtime_support::runtime_v3_telemetry::TelemetryHandle;

const SOURCE_ROOT: &str = "/tmp/sts2-exo-source-b068";
const MODEL_EXECUTION_ID: &str = "execution-11";
const REQUEST_ID: &str = "bootstrap-request";
const TURN_ID: &str = "bootstrap-turn";
struct Fixture {
    root: std::path::PathBuf,
    executor: std::path::PathBuf,
    calls: std::path::PathBuf,
    release: std::path::PathBuf,
    config: RuntimeConfig,
    settings: RuntimeV3Settings,
    durable: DurableHandle,
    authority_state: RuntimeLifecycleAuthorityState,
    request: Value,
}

impl Fixture {
    fn new() -> Self {
        Self::new_with_blocked_effect(false)
    }

    fn new_with_blocked_effect(blocked_effect: bool) -> Self {
        let root = scratch_root();
        std::fs::create_dir(&root).expect("private scratch root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("private permissions");
        let source = std::path::PathBuf::from(SOURCE_ROOT);
        let revision = std::process::Command::new("/usr/bin/git")
            .args(["-C", SOURCE_ROOT, "rev-parse", "HEAD"])
            .output()
            .expect("Exo source revision");
        assert!(revision.status.success());
        assert_eq!(
            revision.stdout,
            format!("{EXO_SOURCE_REVISION}\n").as_bytes()
        );

        let calls = root.join("effect-started");
        let release = root.join("release-effect");
        let executor = root.join("fixture-executor");
        let output = json!({
            "wire_version":"sts2.exo-bridge-wire-v2",
            "request_id":REQUEST_ID,
            "turn_id":TURN_ID,
            "outcome":"decision",
            "decision":{
                "decision":"action",
                "action_id":"combat.end-turn",
                "rationale":"offline lifecycle fixture",
                "confidence":90
            },
            "error_code":null,
            "native":{
                "agent_id":"fixture-agent",
                "conversation_id":"fixture-conversation",
                "session_id":"fixture-session",
                "turn_id":"fixture-native-turn",
                "event_cursor":"fixture-event"
            }
        });
        let wait_for_release = if blocked_effect {
            format!(
                "while [ ! -e '{}' ]; do sleep 0.01; done\n",
                release.display()
            )
        } else {
            String::new()
        };
        std::fs::write(
            &executor,
            format!(
                "#!/bin/sh\necho started >> '{}'\ncat >/dev/null\n{}printf '%s' '{}'\n",
                calls.display(),
                wait_for_release,
                output
            ),
        )
        .expect("fixture executor");
        std::fs::set_permissions(&executor, std::fs::Permissions::from_mode(0o700))
            .expect("fixture executor mode");
        let extension = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../experiments/exo-agent/extension/src/index.ts");
        let node = std::path::PathBuf::from("/usr/local/bin/node");
        let config_path = root.join("bridge.json");
        let config_value = json!({
            "schema":"sts2.exo-one-shot-config-v1",
            "executor":executor,
            "executor_sha256":sha256_hex(std::fs::read(&executor).expect("executor bytes")),
            "source_root":source,
            "extension":extension,
            "extension_sha256":sha256_hex(std::fs::read(&extension).expect("extension bytes")),
            "node":node,
            "node_sha256":sha256_hex(std::fs::read(&node).expect("Node bytes")),
            "model":"o3-pro",
            "endpoint":"https://api.openai.com/v1"
        });
        let config_bytes = serde_json::to_vec(&config_value).expect("bridge configuration");
        std::fs::write(&config_path, &config_bytes).expect("configuration file");
        let config_digest = sha256_hex(config_bytes);
        let process = ExoProcessConfig::new(
            executor.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                config_path.to_string_lossy().into_owned(),
                config_digest,
            ],
            None,
            Vec::new(),
        )
        .expect("bridge process");
        let config = runtime_config();
        let inspected =
            sts2_harness::exo_bridge_configuration::load(process.arguments()[1].as_str())
                .expect("inspected bridge configuration");
        let identity = inspected
            .inspected_identity(
                std::path::Path::new(process.executable()),
                &config.instance_id,
            )
            .expect("inspected deployment identity");
        let trusted = ExoTrustedConfiguration {
            identity: identity.clone(),
            platform: ExoPlatform::LinuxX86_64,
            profile: ExoProfile::Standard,
            context_mode: ExoContextMode::Fresh,
            runtime: ExoRuntime::Responses,
            limits: ExoLimits::reviewed(),
            restricted: ExoRestrictedProfile::reviewed_private(
                "/var/lib/sts2-harness/lifecycle-offline-test",
            ),
        };
        let admission = ExoRuntimeAdmission::Enveloped(Box::new(ExoAdmissionPlan::new(
            trusted,
            identity,
            MODEL_EXECUTION_ID.to_owned(),
            REQUEST_ID.to_owned(),
            TURN_ID.to_owned(),
        )));
        let (lifecycle, secrets) =
            RuntimeLifecycleConfig::bootstrap_test(root.join("journal"), root.join("policy.bin"));
        adopt_fixture_policy(&config, &lifecycle, &secrets, &process);
        let settings = RuntimeV3Settings {
            runner: sts2_harness::EpisodeRunnerConfig::new(
                1,
                sts2_harness::StabilityBarrier::new(1, 1).expect("barrier"),
                sts2_harness::RecoveryController::new(1).expect("recovery"),
                "objective",
                Vec::new(),
            )
            .expect("runner"),
            exo: ExoConfig::new(EXO_SOURCE_REVISION, 8192, 8192, 1000).expect("Exo config"),
            process,
            admission,
            lifecycle: Some((lifecycle, secrets)),
        };
        let config_digest = sha256_hex("config");
        let fingerprint =
            ExecutionFingerprint::new("seed", "build", "state", config_digest.clone(), "provider")
                .expect("fingerprint");
        let lineage = ExecutionLineage::new(
            config.run_id.clone(),
            config.episode_id.clone(),
            "bootstrap-attempt",
            config.trajectory_id.clone(),
        )
        .expect("lineage");
        let mut store = ExecutionStore::open_in_memory().expect("execution store");
        store
            .start_episode(&lineage, &fingerprint)
            .expect("episode admission");
        let durable = DurableHandle::from_store_for_lifecycle_test(
            store,
            lineage,
            fingerprint,
            EXO_SOURCE_REVISION.to_owned(),
            config_digest,
        )
        .expect("durable");
        let authority_state = RuntimeLifecycleAuthorityState::default();
        authority_state
            .enable()
            .expect("enable lifecycle authority");
        let allocation = json!({
            "status":"allocated",
            "instance_id":config.instance_id,
            "caller_id":config.caller_id,
            "session_id":config.session_id,
            "lease_id":config.lease_id,
            "lease_epoch":config.lease_epoch
        });
        let reservation = allocation_context::validate(&allocation, &config)
            .expect("validated gateway reservation");
        let mut acquired_config = config.clone();
        reservation.apply_current_lease(&mut acquired_config);
        authority_state
            .activate(&acquired_config.lease_id, acquired_config.lease_epoch)
            .expect("install reserved lease authority");
        let mut port = RuntimeV3Port::new_with_store(
            acquired_config,
            TelemetryHandle::disabled(),
            durable.clone(),
        )
        .expect("runtime port");
        port.lifecycle_authority = authority_state.clone();
        let mut observed: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))
        .expect("MCP observation fixture");
        observed["correlation_id"] = json!("1");
        observed["instance_id"] = json!(config.instance_id);
        observed["session_id"] = json!(config.session_id);
        observed["lease_id"] = json!(config.lease_id);
        observed["lease_epoch"] = json!(config.lease_epoch);
        observed["legal_actions"] = json!([
            {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
        ]);
        let parsed = parse::observation_with_text(
            &observed,
            &observed.to_string(),
            "state_response",
            &config,
        )
        .expect("validated MCP observation");
        port.install(parsed)
            .expect("install durable MCP observation");
        drop(port);

        let mut request: Value = serde_json::from_slice(include_bytes!(
            "../../../../../protocol-artifact/exo-bridge-v1/golden/request.json"
        ))
        .expect("Exo request fixture");
        request["model_execution_id"] = json!(MODEL_EXECUTION_ID);
        request["provider_revision"] = json!(EXO_SOURCE_REVISION);
        Self {
            root,
            executor,
            calls,
            release,
            config,
            settings,
            durable,
            authority_state,
            request,
        }
    }

    fn admit(&self) -> Result<RuntimeTransport, String> {
        super::admit(
            &self.config,
            &self.settings,
            self.durable.clone(),
            self.authority_state.clone(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.durable.close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn adopt_fixture_policy(
    config: &RuntimeConfig,
    lifecycle: &RuntimeLifecycleConfig,
    secrets: &RuntimeLifecycleSecrets,
    process: &ExoProcessConfig,
) {
    let loaded = sts2_harness::exo_bridge_configuration::load(process.arguments()[1].as_str())
        .expect("policy bridge inspection");
    let identity = loaded
        .inspected_identity(
            std::path::Path::new(process.executable()),
            &config.instance_id,
        )
        .expect("policy identity");
    let capabilities = lifecycle.capabilities(&identity).expect("capabilities");
    let scope = SessionScope::new(
        lifecycle.project_id.clone(),
        config.run_id.clone(),
        config.episode_id.clone(),
        lifecycle.agent_id.clone(),
    )
    .expect("session scope");
    let owner = ProviderSessionPolicyOwner::open(
        ProviderSessionMetadataStore::encrypted(
            &lifecycle.policy_store_path,
            secrets.policy_key,
            scope.clone(),
        )
        .expect("policy store"),
        scope.clone(),
        capabilities.clone(),
    )
    .expect("policy owner");
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = String::from("bootstrap-realm");
    policy.profile_sha256 = capabilities.profile_sha256;
    let digest = owner
        .import(serde_json::to_vec(&policy).expect("policy bytes"))
        .expect("policy import");
    owner.adopt_imported(&digest, 2).expect("policy adoption");
    drop(owner);
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:1"),
        gateway_token: String::from("bootstrap-token"),
        mcp_binary: String::from("unused-mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("bootstrap-instance"),
        caller_id: String::from("bootstrap-caller"),
        session_id: String::from("bootstrap-session"),
        lease_id: String::from("bootstrap-lease"),
        lease_epoch: 1,
        mcp_session_id: String::from("bootstrap-mcp"),
        run_id: String::from("bootstrap-run"),
        episode_id: String::from("bootstrap-episode"),
        trajectory_id: String::from("bootstrap-trajectory"),
        trace_id: String::from("bootstrap-trace"),
        artifact_id: String::from("bootstrap-artifact"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 1,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    }
}

fn scratch_root() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "sts2-lifecycle-bootstrap-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[path = "runtime_v3_lifecycle_admission_tests.rs"]
mod admission_tests;
