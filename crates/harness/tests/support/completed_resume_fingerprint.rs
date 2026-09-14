// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use sts2_harness::ExecutionFingerprint;

pub(crate) fn fingerprint(
    mcp: &Path,
    bridge: &Path,
    gateway_address: &str,
    source_revision: &str,
    run_id: &str,
    episode_id: &str,
    trajectory_id: &str,
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
        "run_id": run_id,
        "episode_id": episode_id,
        "trajectory_id": trajectory_id,
        "trace_id": "trace-completed-resume",
        "artifact_id": "artifact-completed-resume",
        "settlement_timeout_seconds": 30,
        "exo_revision": source_revision,
        "exo_identity": exo_identity(source_revision),
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
        digest_value(&exo_identity(source_revision))?,
    )
    .map_err(|error| format!("fixture fingerprint is invalid: {error}"))
}

fn exo_identity(source_revision: &str) -> Value {
    json!({
        "contract_version": "sts2-exo-bridge-v1",
        "source_revision": source_revision,
        "package_digest": null,
        "extension_digest": null,
        "bridge_digest": null,
        "model_binding": null,
        "prompt_digest": null,
        "tool_digest": null,
        "config_digest": null,
        "native_instance_id": null,
    })
}

fn digest_bytes(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}

fn digest_value(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize fixture config: {error}"))?;
    Ok(digest_bytes(&bytes))
}
