// SPDX-License-Identifier: MIT

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{
    Checkpoint, CompletionRecord, CompletionStatus, ExecutionFingerprint, ExecutionLineage,
    ExecutionStore, ExecutionStoreConfig,
};

const EXO_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const RUN_ID: &str = "run-completed-resume";
const EPISODE_ID: &str = "episode-completed-resume";
const ATTEMPT_ID: &str = "attempt-completed-resume";
const TRAJECTORY_ID: &str = "trajectory-completed-resume";

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    mcp: PathBuf,
    bridge: PathBuf,
    counter: PathBuf,
    lineage: ExecutionLineage,
    fingerprint: ExecutionFingerprint,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn completed_resume_returns_the_stored_record_without_external_calls() -> Result<(), String> {
    let fixture = Fixture::new("completed")?;
    let completion = seed_completed(&fixture)?;
    let output = fixture.command().output().map_err(command_error)?;
    assert_success(&output)?;
    assert_completion_output(&output, &completion)?;
    assert!(
        !fixture.counter.exists(),
        "completed resume invoked a boundary"
    );

    let mut missing_seed = fixture.command();
    missing_seed
        .env_remove("STS2_SEED")
        .env_remove("STS2_VISIBLE_SEED");
    let missing_output = missing_seed.output().map_err(command_error)?;
    assert_failure_contains(&missing_output, "requires STS2_SEED or STS2_VISIBLE_SEED")?;
    assert!(
        !fixture.counter.exists(),
        "fingerprint rejection invoked a boundary"
    );
    Ok(())
}

#[test]
fn interrupted_unknown_resume_is_denied_before_external_calls() -> Result<(), String> {
    let fixture = Fixture::new("interrupted")?;
    let mut store = open_store(&fixture.store)?;
    store
        .start_episode(&fixture.lineage, &fixture.fingerprint)
        .map_err(|error| format!("cannot seed interrupted episode: {error}"))?;
    store
        .mark_interrupted_unknown(&fixture.lineage.episode_id, "test interruption")
        .map_err(|error| format!("cannot mark interrupted episode: {error}"))?;
    store
        .close()
        .map_err(|error| format!("cannot close interrupted store: {error}"))?;

    let output = fixture.command().output().map_err(command_error)?;
    assert_failure_contains(&output, "requires a separately approved reconstruction")?;
    assert!(
        !fixture.counter.exists(),
        "interrupted resume invoked a boundary"
    );
    Ok(())
}

impl Fixture {
    fn new(label: &str) -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock is before epoch: {error}"))?
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sts2-completed-resume-{label}-{nonce}"));
        fs::create_dir(&root)
            .map_err(|error| format!("cannot create fixture directory: {error}"))?;
        let store = root.join("execution.sqlite3");
        let mcp = root.join("mcp-probe.sh");
        let bridge = root.join("provider-probe.sh");
        let counter = root.join("boundary-calls.log");
        write_probe(&mcp, "mcp", &counter)?;
        write_probe(&bridge, "provider", &counter)?;
        let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, ATTEMPT_ID, TRAJECTORY_ID)
            .map_err(|error| format!("fixture lineage is invalid: {error}"))?;
        let fingerprint = fingerprint(&mcp, &bridge)?;
        Ok(Self {
            root,
            store,
            mcp,
            bridge,
            counter,
            lineage,
            fingerprint,
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .arg("--resume")
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_GATEWAY_ADDR", "127.0.0.1:1")
            .env("STS2_GATEWAY_TOKEN", "test-token")
            .env("STS2_MCP_BINARY", &self.mcp)
            .env("STS2_INSTANCE_ID", "instance-completed-resume")
            .env("STS2_CALLER_ID", "caller-completed-resume")
            .env("STS2_SESSION_ID", "session-completed-resume")
            .env("STS2_LEASE_ID", "lease-completed-resume")
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_MCP_SESSION_ID", "mcp-completed-resume")
            .env("STS2_RUN_ID", RUN_ID)
            .env("STS2_EPISODE_ID", EPISODE_ID)
            .env("STS2_ATTEMPT_ID", ATTEMPT_ID)
            .env("STS2_TRAJECTORY_ID", TRAJECTORY_ID)
            .env("STS2_TRACE_ID", "trace-completed-resume")
            .env("STS2_ARTIFACT_ID", "artifact-completed-resume")
            .env("STS2_EXECUTION_STORE_PATH", &self.store)
            .env("STS2_SEED", "seed-completed-resume")
            .env("STS2_BUILD_DIGEST", "build-completed-resume")
            .env("STS2_STATE_DIGEST", "state-completed-resume")
            .env("STS2_EXO_REVISION", EXO_REVISION)
            .env("STS2_EXO_BRIDGE_BINARY", &self.bridge)
            .env("STS2_EXO_FORWARD_VISIBLE_SEED", "true")
            .env("STS2_OBJECTIVE", "complete the test episode")
            .env_remove("STS2_PROVIDER_KIND")
            .env_remove("STS2_LIVE_EPISODE")
            .env_remove("STS2_COMBAT_DEMO")
            .env_remove("STS2_REPLAY_TRAJECTORY");
        command
    }
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

fn fingerprint(mcp: &Path, bridge: &Path) -> Result<ExecutionFingerprint, String> {
    let mcp_bytes = fs::read(mcp).map_err(|error| format!("cannot read MCP probe: {error}"))?;
    let mcp_value = json!({
        "path": mcp.display().to_string(),
        "sha256": digest_bytes(&mcp_bytes),
        "bytes": mcp_bytes.len(),
    });
    let config = json!({
        "runtime_profile": "runtime-v3-gameplay",
        "gateway_address": "127.0.0.1:1",
        "mcp_binary": mcp.display().to_string(),
        "mcp_executable": mcp_value,
        "instance_id": "instance-completed-resume",
        "caller_id": "caller-completed-resume",
        "session_id": "session-completed-resume",
        "lease_id": "lease-completed-resume",
        "lease_epoch": 1,
        "mcp_session_id": "mcp-completed-resume",
        "run_id": RUN_ID,
        "episode_id": EPISODE_ID,
        "trajectory_id": TRAJECTORY_ID,
        "trace_id": "trace-completed-resume",
        "artifact_id": "artifact-completed-resume",
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
        "seed-completed-resume",
        "build-completed-resume",
        "state-completed-resume",
        digest_value(&config)?,
        EXO_REVISION,
    )
    .map_err(|error| format!("fixture fingerprint is invalid: {error}"))
}

fn seed_completed(fixture: &Fixture) -> Result<CompletionRecord, String> {
    let mut store = open_store(&fixture.store)?;
    store
        .start_episode(&fixture.lineage, &fixture.fingerprint)
        .map_err(|error| format!("cannot seed episode: {error}"))?;
    let checkpoint = Checkpoint::new(
        fixture.lineage.clone(),
        0,
        "state-terminal",
        3,
        fixture.fingerprint.clone(),
        br#"{"terminal":true}"#.to_vec(),
        "legal-actions-digest",
    )
    .map_err(|error| format!("checkpoint is invalid: {error}"))?;
    store
        .save_checkpoint(&checkpoint)
        .map_err(|error| format!("cannot seed checkpoint: {error}"))?;
    let completion = CompletionRecord::new(
        fixture.lineage.clone(),
        CompletionStatus::Completed,
        "terminal-victory-3",
        0,
        "result-digest-completed-resume",
    )
    .map_err(|error| format!("completion is invalid: {error}"))?;
    store
        .record_completion(&completion)
        .map_err(|error| format!("cannot seed completion: {error}"))?;
    store
        .close()
        .map_err(|error| format!("cannot close completed store: {error}"))?;
    Ok(completion)
}

fn open_store(path: &Path) -> Result<ExecutionStore, String> {
    ExecutionStore::open(ExecutionStoreConfig::new(path))
        .map_err(|error| format!("cannot open fixture store: {error}"))
}

fn assert_success(output: &Output) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "runtime child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn assert_completion_output(output: &Output, completion: &CompletionRecord) -> Result<(), String> {
    if output.stdout.iter().filter(|byte| **byte == b'\n').count() != 1 {
        return Err(format!(
            "runtime child emitted unexpected stdout: {:?}",
            output.stdout
        ));
    }
    let actual: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("runtime child output is not JSON: {error}"))?;
    let expected = json!({
        "lineage": {
            "run_id": completion.lineage.run_id,
            "episode_id": completion.lineage.episode_id,
            "attempt_id": completion.lineage.attempt_id,
            "trajectory_id": completion.lineage.trajectory_id,
        },
        "status": "completed",
        "terminal_ref": completion.terminal_ref,
        "checkpoint_sequence": completion.checkpoint_sequence,
        "result_digest": completion.result_digest,
    });
    if actual != expected {
        return Err(format!(
            "runtime child returned {actual}, expected {expected}"
        ));
    }
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

fn command_error(error: std::io::Error) -> String {
    format!("cannot spawn runtime child: {error}")
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_value(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize fixture config: {error}"))?;
    Ok(digest_bytes(&bytes))
}
