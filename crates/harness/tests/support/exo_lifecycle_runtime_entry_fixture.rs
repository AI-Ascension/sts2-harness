// SPDX-License-Identifier: MIT

use serde_json::json;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use sts2_harness::exo_lifecycle::EXO_LIFECYCLE_WIRE_V2;
use sts2_harness::provider_session::{
    ProviderSessionMetadataStore, ProviderSessionMode, ProviderSessionPolicy,
    ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::{EXO_SOURCE_REVISION, sha256_hex};

#[path = "exo_lifecycle_runtime_entry_fixture_helpers.rs"]
mod helpers;
pub(super) use helpers::lifecycle_capabilities;
use helpers::{required_axis, write_executable};

use super::peers;
use super::{
    ATTEMPT_ID, CALLER_ID, EPISODE_ID, EXO_SOURCE, INSTANCE_ID, LEASE_ID, REQUEST_ID, RUN_ID,
    SESSION_ID, TRAJECTORY_ID, TURN_ID,
};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) gateway: Option<TcpListener>,
    pub(super) gateway_address: String,
    pub(super) mcp: PathBuf,
    pub(super) executor: PathBuf,
    pub(super) bridge_config: PathBuf,
    pub(super) policy_store: PathBuf,
    pub(super) journal: PathBuf,
    pub(super) action_log: PathBuf,
    pub(super) effect_log: PathBuf,
    pub(super) input_log: PathBuf,
    pub(super) execution_store: PathBuf,
    pub(super) inspected_identity: sts2_harness::ExoIdentity,
    pub(super) bridge_config_digest: String,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sts2-exo-lifecycle-entry-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).map_err(|error| error.to_string())?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        let gateway = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let gateway_address = gateway
            .local_addr()
            .map_err(|error| error.to_string())?
            .to_string();
        let mcp = root.join("mcp-peer.py");
        let executor = root.join("inert-executor");
        let bridge_config = root.join("bridge.json");
        let policy_store = root.join("policy.enc");
        let journal = root.join("lifecycle-journal");
        let action_log = root.join("dispatched-action.json");
        let effect_log = root.join("provider-effect-started");
        let input_log = root.join("provider-input.bin");
        let execution_store = root.join("execution.sqlite3");

        let extension = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../experiments/exo-agent/extension/src/index.ts");
        let node = PathBuf::from("/usr/local/bin/node");
        let executor_script = format!(
            "#!/bin/sh\ncat > '{}'\nprintf 'provider-effect\\n' >> '{}'\nprintf '%s' '{}'\n",
            input_log.display(),
            effect_log.display(),
            json!({
                "wire_version": EXO_LIFECYCLE_WIRE_V2,
                "request_id": REQUEST_ID,
                "turn_id": TURN_ID,
                "outcome": "decision",
                "decision": {
                    "decision": "action",
                    "action_id": "combat.end-turn",
                    "rationale": "offline entrypoint fixture",
                    "confidence": 90
                },
                "error_code": null,
                "native": {
                    "agent_id": "entry-agent",
                    "conversation_id": "entry-conversation",
                    "session_id": "entry-native-session",
                    "turn_id": "entry-native-turn",
                    "event_cursor": "entry-event"
                }
            })
        );
        write_executable(&executor, &executor_script)?;
        let bridge_value = json!({
            "schema":"sts2.exo-one-shot-config-v1",
            "executor":executor,
            "executor_sha256":sha256_hex(std::fs::read(&executor).map_err(|error| error.to_string())?),
            "source_root":EXO_SOURCE,
            "extension":extension,
            "extension_sha256":sha256_hex(std::fs::read(&extension).map_err(|error| error.to_string())?),
            "node":node,
            "node_sha256":sha256_hex(std::fs::read(&node).map_err(|error| error.to_string())?),
            "model":"o3-pro",
            "endpoint":"https://api.openai.com/v1"
        });
        let bridge_bytes = serde_json::to_vec(&bridge_value).map_err(|error| error.to_string())?;
        std::fs::write(&bridge_config, &bridge_bytes).map_err(|error| error.to_string())?;
        let bridge_config_digest = sha256_hex(&bridge_bytes);
        let process = sts2_harness::ExoProcessConfig::new(
            executor.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                bridge_config.to_string_lossy().into_owned(),
                bridge_config_digest.clone(),
            ],
            None,
            Vec::new(),
        )
        .map_err(|error| error.to_string())?;
        let inspected =
            sts2_harness::exo_bridge_configuration::load(process.arguments()[1].as_str())
                .map_err(|error| format!("fixture bridge did not inspect: {error}"))?;
        let inspected_identity = inspected
            .inspected_identity(Path::new(process.executable()), INSTANCE_ID)
            .map_err(|error| format!("fixture identity did not inspect: {error}"))?;
        let script = peers::mcp_script(&action_log)?;
        write_executable(&mcp, &script)?;

        Ok(Self {
            root,
            gateway: Some(gateway),
            gateway_address,
            mcp,
            executor,
            bridge_config,
            policy_store,
            journal,
            action_log,
            effect_log,
            input_log,
            execution_store,
            inspected_identity,
            bridge_config_digest,
        })
    }

    pub(super) fn prepare_policy(&self) -> Result<(), String> {
        let scope = SessionScope::new("entry-project", RUN_ID, EPISODE_ID, "entry-agent")
            .map_err(|error| error.to_string())?;
        let capabilities = lifecycle_capabilities(&self.inspected_identity)?;
        let store =
            ProviderSessionMetadataStore::encrypted(&self.policy_store, [0x22; 32], scope.clone())
                .map_err(|error| error.to_string())?;
        let owner = ProviderSessionPolicyOwner::open(store, scope.clone(), capabilities.clone())
            .map_err(|error| error.to_string())?;
        let mut policy = ProviderSessionPolicy::disabled(scope);
        policy.mode = ProviderSessionMode::FixtureOnly;
        policy.credential_realm_ref = String::from("entry-fixture-realm");
        policy.profile_sha256 = capabilities.profile_sha256;
        let bytes = serde_json::to_vec(&policy).map_err(|error| error.to_string())?;
        let digest = owner.import(bytes).map_err(|error| error.to_string())?;
        owner
            .adopt_imported(&digest, 2)
            .map_err(|error| error.to_string())?;
        drop(owner);
        Ok(())
    }

    pub(super) fn command(&self) -> Result<Command, String> {
        let configuration = json!({
            "schema_version":"sts2.exo-lifecycle-runtime-v1",
            "directory":self.journal,
            "store_id":"entry-journal",
            "key_reference":"STS2_TEST_LIFECYCLE_KEY",
            "owner_token_reference":"STS2_TEST_LIFECYCLE_OWNER",
            "project_id":"entry-project",
            "agent_id":"entry-agent",
            "policy_store_path":self.policy_store,
            "policy_key_reference":"STS2_TEST_POLICY_KEY",
            "legacy_path":null
        });
        let arguments = vec![
            String::from("--run-v2"),
            self.bridge_config.to_string_lossy().into_owned(),
            self.bridge_config_digest.clone(),
        ];
        let identity = &self.inspected_identity;
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_GATEWAY_ADDR", &self.gateway_address)
            .env("STS2_GATEWAY_TOKEN", "offline-entry-token")
            .env("STS2_MCP_BINARY", &self.mcp)
            .env("STS2_INSTANCE_ID", INSTANCE_ID)
            .env("STS2_CALLER_ID", CALLER_ID)
            .env("STS2_SESSION_ID", SESSION_ID)
            .env("STS2_LEASE_ID", LEASE_ID)
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_MCP_SESSION_ID", "entry-mcp-session")
            .env("STS2_RUN_ID", RUN_ID)
            .env("STS2_EPISODE_ID", EPISODE_ID)
            .env("STS2_ATTEMPT_ID", ATTEMPT_ID)
            .env("STS2_TRAJECTORY_ID", TRAJECTORY_ID)
            .env("STS2_TRACE_ID", "entry-trace")
            .env("STS2_ARTIFACT_ID", "entry-artifact")
            .env("STS2_SEED", "entry-seed")
            .env("STS2_BUILD_DIGEST", "entry-build")
            .env("STS2_STATE_DIGEST", "entry-state")
            .env("STS2_EXECUTION_STORE_PATH", &self.execution_store)
            .env("STS2_EXO_REVISION", EXO_SOURCE_REVISION)
            .env("STS2_EXO_ADMISSION", "envelope")
            .env("STS2_EXO_BRIDGE_BINARY", &self.executor)
            .env(
                "STS2_EXO_BRIDGE_ARGS_JSON",
                serde_json::to_string(&arguments).map_err(|error| error.to_string())?,
            )
            .env("STS2_EXO_INHERITED_ENV_JSON", "[]")
            .env("STS2_EXO_PACKAGE_PATH", &self.executor)
            .env(
                "STS2_EXO_PACKAGE_DIGEST",
                required_axis(identity.package_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_EXTENSION_DIGEST",
                required_axis(identity.extension_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_BRIDGE_DIGEST",
                required_axis(identity.bridge_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_MODEL_BINDING",
                required_axis(identity.model_binding.as_deref())?,
            )
            .env(
                "STS2_EXO_PROVIDER",
                required_axis(identity.provider.as_deref())?,
            )
            .env(
                "STS2_EXO_ENDPOINT",
                required_axis(identity.endpoint.as_deref())?,
            )
            .env(
                "STS2_EXO_PROMPT_DIGEST",
                required_axis(identity.prompt_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_TOOL_DIGEST",
                required_axis(identity.tool_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_CONFIG_DIGEST",
                required_axis(identity.config_digest.as_deref())?,
            )
            .env(
                "STS2_EXO_NATIVE_INSTANCE_ID",
                required_axis(identity.native_instance_id.as_deref())?,
            )
            .env("STS2_EXO_MODEL_EXECUTION_ID", "model-execution-1")
            .env("STS2_EXO_REQUEST_ID", REQUEST_ID)
            .env("STS2_EXO_TURN_ID", TURN_ID)
            .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
            .env("STS2_EXO_TIMEOUT_MILLIS", "5000")
            .env("STS2_EXO_LIFECYCLE_CONFIG", configuration.to_string())
            .env("STS2_TEST_LIFECYCLE_KEY", "11".repeat(32))
            .env("STS2_TEST_LIFECYCLE_OWNER", "entry-owner-token")
            .env("STS2_TEST_POLICY_KEY", "22".repeat(32))
            .env("STS2_OBJECTIVE", "complete offline lifecycle fixture")
            .env("STS2_MAX_STEPS", "4")
            .env("STS2_BARRIER_MAX_POLLS", "1")
            .env("STS2_BARRIER_WAIT_MILLIS", "1")
            .env("STS2_RECOVERY_MAX_ATTEMPTS", "1");
        Ok(command)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
