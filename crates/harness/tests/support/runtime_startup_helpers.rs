// SPDX-License-Identifier: MIT

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;

use rusqlite::{Connection, params, types::Value as SqlValue};
use serde_json::{Value, json};
use sts2_harness::{ExecutionFingerprint, ExecutionStore, ExecutionStoreConfig, OperationIntent};

use super::{EPISODE_ID, EXO_REVISION, Fixture, RUN_ID, TRAJECTORY_ID, run_child};

pub(super) fn write_probe(path: &Path, marker: &str, counter: &Path) -> Result<(), String> {
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
        "exo_identity": {
            "contract_version": "sts2-exo-bridge-v1",
            "source_revision": EXO_REVISION,
            "package_digest": null,
            "extension_digest": null,
            "bridge_digest": null,
            "model_binding": null,
            "prompt_digest": null,
            "tool_digest": null,
            "config_digest": null,
            "native_instance_id": null,
        },
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
    let provider_identity = json!({
        "contract_version": "sts2-exo-bridge-v1",
        "source_revision": EXO_REVISION,
        "package_digest": null,
        "extension_digest": null,
        "bridge_digest": null,
        "model_binding": null,
        "prompt_digest": null,
        "tool_digest": null,
        "config_digest": null,
        "native_instance_id": null,
    });
    ExecutionFingerprint::new(
        "seed-runtime-startup-hostile",
        "build-runtime-startup-hostile",
        "state-runtime-startup-hostile",
        digest_value(&config)?,
        digest_value(&provider_identity)?,
    )
    .map_err(|error| format!("fixture fingerprint is invalid: {error}"))
}

pub(super) fn seed_hostile_operation(fixture: &Fixture) -> Result<(), String> {
    seed_valid_operation(fixture)?;
    rewrite_operation_payload(
        fixture,
        &vec![b'x'; sts2_harness::MAX_OPERATION_ACTION_BYTES + 1],
        "payload-digest",
    )
}

pub(super) fn seed_matching_digest_malformed_operation(fixture: &Fixture) -> Result<(), String> {
    seed_valid_operation(fixture)?;
    let malformed = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn""#;
    rewrite_operation_payload(fixture, malformed, &digest_bytes(malformed))
}

pub(super) fn seed_mismatched_action_digest_operation(fixture: &Fixture) -> Result<(), String> {
    seed_valid_operation(fixture)?;
    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
    rewrite_operation_payload(fixture, canonical, &"0".repeat(64))
}

pub(super) fn assert_hostile_startup_is_bounded(fixture: &Fixture) -> Result<(), String> {
    let output = run_child(fixture.command())?;
    assert_failure_contains(&output, "cannot inspect runtime-v3 execution state")?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "hostile startup state invoked an MCP or provider boundary",
        ));
    }
    Ok(())
}

pub(super) fn assert_failure_contains(output: &Output, expected: &str) -> Result<(), String> {
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

pub(super) fn digest_bytes(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}

fn seed_valid_operation(fixture: &Fixture) -> Result<(), String> {
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
        "11111111-1111-4111-8111-111111111111",
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
    Ok(())
}

fn rewrite_operation_payload(
    fixture: &Fixture,
    payload: &[u8],
    digest: &str,
) -> Result<(), String> {
    let connection = Connection::open(&fixture.store)
        .map_err(|error| format!("cannot reopen fixture store: {error}"))?;
    connection
        .execute(
            "UPDATE operations SET action_payload = ?1, payload_digest = ?2
             WHERE operation_id = '11111111-1111-4111-8111-111111111111'",
            params![SqlValue::Blob(payload.to_vec()), digest],
        )
        .map_err(|error| format!("cannot poison operation: {error}"))?;
    Ok(())
}

fn digest_value(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize fixture config: {error}"))?;
    Ok(digest_bytes(&bytes))
}
