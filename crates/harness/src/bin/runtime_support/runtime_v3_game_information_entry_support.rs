// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};
use sts2_harness::context_memory::policy_owner::SavedPolicyRef;
use sts2_harness::management::ManagementClient;

pub(super) fn free_loopback_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("loopback address")
}

pub(super) fn finish_child(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().expect("poll runtime process").is_some() {
            return child.wait_with_output().expect("collect runtime output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("collect timed-out child");
            panic!(
                "actual runtime entry exceeded its bounded test deadline:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn read_json_lines(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("synthetic process event JSON"))
        .collect()
}

pub(super) fn write_private(path: &std::path::Path, bytes: &str) {
    std::fs::write(path, bytes).expect("write private runtime fixture");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .expect("restrict fixture permissions");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_owner_config(
    path: &std::path::Path,
    corpus_path: &std::path::Path,
    policy_path: &std::path::Path,
    archive_path: &std::path::Path,
    management_address: SocketAddr,
    replay_archive: bool,
) -> String {
    let config = json!({
        "schema": "ascension.runtime-v3.memory-policy-owner-config.v2",
        "scope": {
            "project_id": PROJECT, "run_id": RUN, "episode_id": EPISODE, "agent_id": AGENT
        },
        "corpus_store_path": corpus_path,
        "policy_store_path": policy_path,
        "lookup_archive_store_path": archive_path,
        "replay_archive": replay_archive,
        "archive_retention_seconds": 86400,
        "policy_store_consent_ref": "entry-policy-consent",
        "auth_profile": "lookup-owner",
        "management_listen": management_address.to_string(),
        "preflight_timeout_seconds": 60,
        "selector_grant_id": "policy-grant",
        "operator_grant_id": "policy-grant",
        "phase2_revision_id": "revision-1",
        "control_epoch": 1,
        "plan_epoch": 1,
        "owner_epoch": 1,
        "grants": [{
            "grant_id": "policy-grant",
            "subject": "profile:lookup-owner",
            "permissions": [
                "read_metadata", "read_content", "write", "approve", "adopt", "select"
            ],
            "epoch": 1,
            "expires_at": 4102444800_u64,
            "revoked": false
        }]
    });
    let bytes = serde_json::to_vec(&config).expect("encode closed owner config");
    std::fs::write(path, &bytes).expect("write owner config");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .expect("private owner config");
    sts2_harness::sha256_hex(bytes)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_runtime_child(
    gateway_address: SocketAddr,
    config_path: &std::path::Path,
    config_sha: &str,
    mcp_binary: &std::path::Path,
    runtime_binary: Option<&std::path::Path>,
    gateway_token: &str,
    agent_script: &std::path::Path,
    python: &std::path::Path,
    python_sha: &str,
    agent_log: &std::path::Path,
    execution_path: &std::path::Path,
    branch_path: &std::path::Path,
    replay: bool,
) -> Child {
    let args = vec![agent_script.to_string_lossy().to_string()];
    let args_json = serde_json::to_string(&args).expect("lookup process args");
    let mut command = match runtime_binary {
        Some(binary) => Command::new(binary),
        None => Command::new(std::env::current_exe().expect("current test binary")),
    };
    command.env_clear().env("PATH", "/usr/bin:/bin");
    if runtime_binary.is_none() {
        command
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--nocapture")
            .env("STS2_LOOKUP_ENTRY_CHILD", "true");
    }
    command
        .env("STS2_RUNTIME_PROFILE", "negotiated-composition-v1")
        .env("STS2_LIVE_EPISODE", "true")
        .env("STS2_ENABLE_GAME_INFORMATION_LOOKUP_BINDING", "true")
        .env("STS2_GATEWAY_ADDR", gateway_address.to_string())
        .env("STS2_GATEWAY_TOKEN", gateway_token)
        .env("STS2_MCP_BINARY", mcp_binary)
        .env("STS2_INSTANCE_ID", "instance-1")
        .env("STS2_CALLER_ID", "harness")
        .env("STS2_SESSION_ID", "session-1")
        .env("STS2_LEASE_ID", "lease-1")
        .env("STS2_LEASE_EPOCH", "1")
        .env("STS2_MCP_SESSION_ID", "mcp-session-1")
        .env("STS2_RUN_ID", RUN)
        .env("STS2_EPISODE_ID", EPISODE)
        .env("STS2_TRAJECTORY_ID", "entry-trajectory")
        .env("STS2_TRACE_ID", "entry-trace")
        .env("STS2_ARTIFACT_ID", "entry-artifact")
        .env("STS2_PROJECT_ID", PROJECT)
        .env("STS2_AGENT_ID", AGENT)
        .env("STS2_AUTHORITY_EPOCH", "not-the-selected-owner-epoch")
        .env(
            "STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON",
            r#"{"operation":"observe"}"#,
        )
        .env("STS2_OBJECTIVE", "choose one legal action")
        .env("STS2_HARD_CONSTRAINTS_JSON", "[]")
        .env("STS2_MAX_STEPS", "4")
        .env("STS2_RECOVERY_MAX_ATTEMPTS", "1")
        .env("STS2_LOOKUP_OWNER_CONFIG", config_path)
        .env("STS2_LOOKUP_OWNER_CONFIG_SHA256", config_sha)
        .env("STS2_LOOKUP_CORPUS_STORE_KEY_HEX", "09".repeat(32))
        .env("STS2_LOOKUP_POLICY_STORE_KEY_HEX", "07".repeat(32))
        .env("STS2_LOOKUP_ARCHIVE_STORE_KEY_HEX", "0b".repeat(32))
        .env("STS2_WORKFLOW_TOKEN_LOOKUP_OWNER", OWNER_TOKEN)
        .env("STS2_LOOKUP_AGENT_BINARY", python)
        .env("STS2_LOOKUP_AGENT_ARGS_JSON", args_json)
        .env("STS2_LOOKUP_AGENT_SHA256", python_sha)
        .env("STS2_LOOKUP_AGENT_TIMEOUT_MILLIS", "5000")
        .env(
            "STS2_LOOKUP_AGENT_INHERITED_ENV_JSON",
            serde_json::to_string(&["STS2_TEST_AGENT_LOG"]).expect("safe agent log allowlist"),
        )
        .env("STS2_TEST_AGENT_LOG", agent_log)
        .env("STS2_EXECUTION_STORE_PATH", execution_path)
        .env("STS2_BRANCH_STORE_PATH", branch_path)
        .env(
            "STS2_ATTEMPT_ID",
            if replay {
                "entry-attempt-replay"
            } else {
                "entry-attempt-first"
            },
        )
        .spawn()
        .expect("start isolated actual runtime test process")
}

pub(super) fn wait_for_management(address: SocketAddr) -> ManagementClient {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let client = ManagementClient::new(address, OWNER_TOKEN).expect("loopback owner client");
        if let Ok(response) = client.request_json("GET", "/v1/memory-policy-owner", None)
            && response.status == 200
        {
            let status: Value = serde_json::from_slice(&response.body).expect("owner status JSON");
            assert_eq!(
                status["lookup_ready"], false,
                "startup must require adoption"
            );
            return client;
        }
        assert!(
            Instant::now() < deadline,
            "runtime owner management route did not become ready"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn explicit_revalidation_approval_and_adoption(
    client: &ManagementClient,
    source: &SavedPolicyRef,
    source_bytes: &[u8],
    suffix: &str,
) {
    let current = client
        .request_json("GET", "/v1/memory-policy-owner", None)
        .expect("read current active policy binding");
    assert_eq!(current.status, 200);
    let current: Value =
        serde_json::from_slice(&current.body).expect("current policy owner status");
    let expected_active_version = current["active_binding"]["version"]
        .as_u64()
        .expect("current active binding version");
    let review_id = format!("{suffix}-revalidation");
    let path = format!(
        "/v1/memory-policy-owner/proposals/{review_id}?source_policy_id={}&source_version={}&source_raw_sha256={}&expected_active_version={}",
        source.policy_id, source.version, source.raw_sha256, expected_active_version
    );
    let proposal = client
        .request_json_with_idempotency_key(
            "POST",
            &path,
            source_bytes,
            &format!("{suffix}-propose"),
        )
        .expect("propose current exact policy bytes");
    assert_eq!(
        proposal.status,
        200,
        "proposal response: {}",
        String::from_utf8_lossy(&proposal.body)
    );
    let review_response = client
        .request_json(
            "GET",
            &format!("/v1/memory-policy-owner/reviews/{review_id}"),
            None,
        )
        .expect("read current policy review");
    assert_eq!(review_response.status, 200);
    let review: Value = serde_json::from_slice(&review_response.body).expect("policy review JSON");
    assert_eq!(review["kind"], "revalidation");
    assert_eq!(review["violations"], json!([]));
    let digest = review["review_sha256"]
        .as_str()
        .expect("canonical revalidation digest");
    let digest_body =
        serde_json::to_vec(&json!({"review_sha256":digest})).expect("encode review digest");
    let premature = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/adopt"),
            &digest_body,
            &format!("{suffix}-premature-adopt"),
        )
        .expect("premature adoption response");
    assert_eq!(premature.status, 403, "adoption requires explicit approval");
    let approval = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/approve"),
            &digest_body,
            &format!("{suffix}-approve"),
        )
        .expect("approve exact revalidation");
    assert_eq!(approval.status, 200);
    let adopted = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/adopt"),
            &digest_body,
            &format!("{suffix}-adopt"),
        )
        .expect("adopt after explicit approval");
    assert_eq!(adopted.status, 200);
}
