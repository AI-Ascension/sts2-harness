// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::ExecutionFingerprint;

pub(super) fn fingerprint(
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
        "instance_id": "instance-completed-resume",
        "caller_id": "caller-completed-resume",
        "session_id": "session-completed-resume",
        "lease_id": "lease-completed-resume",
        "lease_epoch": 1,
        "mcp_session_id": "mcp-completed-resume",
        "run_id": super::RUN_ID,
        "episode_id": super::EPISODE_ID,
        "trajectory_id": super::TRAJECTORY_ID,
        "trace_id": "trace-completed-resume",
        "artifact_id": "artifact-completed-resume",
        "settlement_timeout_seconds": 30,
        "exo_revision": super::EXO_REVISION,
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
        "workflow_binding": {
            "version": "runtime-v3-workflow-binding-v1",
            "workflow": "full_episode",
            "replay": "none",
            "source_sha256": null,
        },
    });
    ExecutionFingerprint::new(
        "seed-completed-resume",
        "build-completed-resume",
        "state-completed-resume",
        digest_value(&config)?,
        super::EXO_REVISION,
    )
    .map_err(|error| format!("fixture fingerprint is invalid: {error}"))
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_value(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize fixture config: {error}"))?;
    Ok(digest_bytes(&bytes))
}
