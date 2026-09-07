// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params, types::Value as SqlValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, OperationIntent,
};

const EXO_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const RUN_ID: &str = "run-runtime-startup-hostile";
const EPISODE_ID: &str = "episode-runtime-startup-hostile";
const ATTEMPT_ID: &str = "attempt-runtime-startup-hostile";
const TRAJECTORY_ID: &str = "trajectory-runtime-startup-hostile";

#[path = "support/completed_resume_process_support.rs"]
mod process_support;

use process_support::run_child;

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    mcp: PathBuf,
    bridge: PathBuf,
    counter: PathBuf,
    gateway: TcpListener,
    gateway_address: String,
    lineage: ExecutionLineage,
    fingerprint: ExecutionFingerprint,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock is before epoch: {error}"))?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sts2-runtime-startup-hostile-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root)
            .map_err(|error| format!("cannot create fixture directory: {error}"))?;
        let store = root.join("execution.sqlite3");
        let mcp = root.join("mcp-probe.sh");
        let bridge = root.join("provider-probe.sh");
        let counter = root.join("boundary-calls.log");
        let gateway = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("cannot bind gateway probe: {error}"))?;
        gateway
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure gateway probe: {error}"))?;
        let gateway_address = gateway
            .local_addr()
            .map_err(|error| format!("cannot read gateway probe address: {error}"))?
            .to_string();
        write_probe(&mcp, "mcp", &counter)?;
        write_probe(&bridge, "provider", &counter)?;
        let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, ATTEMPT_ID, TRAJECTORY_ID)
            .map_err(|error| format!("fixture lineage is invalid: {error}"))?;
        let fingerprint = fingerprint(&mcp, &bridge, &gateway_address)?;
        Ok(Self {
            root,
            store,
            mcp,
            bridge,
            counter,
            gateway,
            gateway_address,
            lineage,
            fingerprint,
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .env_clear()
            .arg("--resume")
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_GATEWAY_ADDR", &self.gateway_address)
            .env("STS2_GATEWAY_TOKEN", "test-token")
            .env("STS2_MCP_BINARY", &self.mcp)
            .env("STS2_INSTANCE_ID", "instance-runtime-startup-hostile")
            .env("STS2_CALLER_ID", "caller-runtime-startup-hostile")
            .env("STS2_SESSION_ID", "session-runtime-startup-hostile")
            .env("STS2_LEASE_ID", "lease-runtime-startup-hostile")
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_MCP_SESSION_ID", "mcp-runtime-startup-hostile")
            .env("STS2_RUN_ID", RUN_ID)
            .env("STS2_EPISODE_ID", EPISODE_ID)
            .env("STS2_ATTEMPT_ID", ATTEMPT_ID)
            .env("STS2_TRAJECTORY_ID", TRAJECTORY_ID)
            .env("STS2_TRACE_ID", "trace-runtime-startup-hostile")
            .env("STS2_ARTIFACT_ID", "artifact-runtime-startup-hostile")
            .env("STS2_EXECUTION_STORE_PATH", &self.store)
            .env("STS2_SEED", "seed-runtime-startup-hostile")
            .env("STS2_BUILD_DIGEST", "build-runtime-startup-hostile")
            .env("STS2_STATE_DIGEST", "state-runtime-startup-hostile")
            .env("STS2_EXO_REVISION", EXO_REVISION)
            .env("STS2_EXO_BRIDGE_BINARY", &self.bridge)
            .env("STS2_EXO_FORWARD_VISIBLE_SEED", "true")
            .env("STS2_OBJECTIVE", "complete the test episode");
        command
    }

    fn assert_no_gateway_connection(&self) -> Result<(), String> {
        let mut attempts = 0_u32;
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            match self.gateway.accept() {
                Ok((_stream, _address)) => attempts = attempts.saturating_add(1),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("gateway probe failed: {error}")),
            }
        }
        if attempts == 0 {
            Ok(())
        } else {
            Err(format!("runtime made {attempts} gateway TCP connection(s)"))
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn malformed_persisted_operation_fails_before_gateway_mcp_or_provider_calls() -> Result<(), String>
{
    let fixture = Fixture::new()?;
    seed_hostile_operation(&fixture)?;
    let output = run_child(fixture.command())?;
    assert_failure_contains(&output, "cannot inspect runtime-v3 execution state")?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "malformed startup state invoked an MCP or provider boundary",
        ));
    }
    Ok(())
}

fn write_probe(path: &Path, marker: &str, counter: &Path) -> Result<(), String> {
    let body = format!(
        "#!/bin/sh\nprintf '{marker}' >> '{}'\nexit 17\n",
        counter.display()
    );
    fs::write(path, body).map_err(|error| format!("cannot write probe: {error}"))?;
    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("cannot inspect probe: {error}"))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("cannot make probe executable: {error}"))
}

fn fingerprint(
    mcp: &Path,
    bridge: &Path,
    gateway_address: &str,
) -> Result<ExecutionFingerprint, String> {
    let mcp_bytes = fs::read(mcp).map_err(|error| format!("cannot read MCP probe: {error}"))?;
    let mcp_value = json!({
        "path": mcp.display().to_string(),
        "sha256": digest_bytes(&mcp_bytes),
        "bytes": mcp_bytes.len(),
    });
    let config = json!({
        "runtime_profile": "runtime-v3-gameplay",
        "gateway_address": gateway_address,
        "mcp_binary": mcp.display().to_string(),
        "mcp_executable": mcp_value,
        "instance_id": "instance-runtime-startup-hostile",
        "caller_id": "caller-runtime-startup-hostile",
        "session_id": "session-runtime-startup-hostile",
        "lease_id": "lease-runtime-startup-hostile",
        "lease_epoch": 1,
        "mcp_session_id": "mcp-runtime-startup-hostile",
        "run_id": RUN_ID,
        "episode_id": EPISODE_ID,
        "trajectory_id": TRAJECTORY_ID,
        "trace_id": "trace-runtime-startup-hostile",
        "artifact_id": "artifact-runtime-startup-hostile",
        "settlement_timeout_seconds": 30,
        "exo_revision": EXO_REVISION,
        "exo_max_request_bytes": 131072,
        "exo_max_response_bytes": 8192,
        "exo_timeout_millis": 120000,
        "exo_forward_visible_seed": true,
        "exo_bridge": {
            "executable": bridge.display().to_string(),
            "arguments": [],
            "working_directory": null,
            "inherited_environment": [],
        },
        "runner": {
            "max_steps": 1024,
            "objective": "complete the test episode",
            "hard_constraints": [],
        },
    });
    ExecutionFingerprint::new(
        "seed-runtime-startup-hostile",
        "build-runtime-startup-hostile",
        "state-runtime-startup-hostile",
        digest_value(&config)?,
        EXO_REVISION,
    )
    .map_err(|error| format!("fixture fingerprint is invalid: {error}"))
}

fn seed_hostile_operation(fixture: &Fixture) -> Result<(), String> {
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&fixture.store))
        .map_err(|error| format!("cannot open fixture store: {error}"))?;
    store
        .start_episode(&fixture.lineage, &fixture.fingerprint)
        .map_err(|error| format!("cannot seed episode: {error}"))?;
    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
    let payload_digest = digest_bytes(canonical);
    let catalog_digest =
        digest_bytes(br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]"#);
    let intent = OperationIntent::new_with_action(
        fixture.lineage.clone(),
        "operation-hostile",
        "state-hostile",
        1,
        "combat.end-turn",
        "end_turn",
        canonical.to_vec(),
        payload_digest,
        "input-hostile",
        Some(catalog_digest),
    )
    .map_err(|error| format!("hostile intent is invalid: {error}"))?;
    store
        .record_operation_intent(&intent)
        .map_err(|error| format!("cannot seed operation: {error}"))?;
    store
        .close()
        .map_err(|error| format!("cannot close fixture store: {error}"))?;
    let connection = Connection::open(&fixture.store)
        .map_err(|error| format!("cannot reopen fixture store: {error}"))?;
    connection
        .execute(
            "UPDATE operations SET action_payload = ?1 WHERE operation_id = 'operation-hostile'",
            params![SqlValue::Blob(vec![
                b'x';
                sts2_harness::MAX_OPERATION_ACTION_BYTES
                    + 1
            ])],
        )
        .map_err(|error| format!("cannot poison operation: {error}"))?;
    Ok(())
}

fn assert_failure_contains(output: &Output, expected: &str) -> Result<(), String> {
    if output.status.success() {
        return Err(String::from("runtime child unexpectedly succeeded"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(expected) {
        Ok(())
    } else {
        Err(format!(
            "runtime child error {stderr:?} omitted {expected:?}"
        ))
    }
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_value(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize fixture config: {error}"))?;
    Ok(digest_bytes(&bytes))
}
