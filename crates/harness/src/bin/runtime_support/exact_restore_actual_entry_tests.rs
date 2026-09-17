// SPDX-License-Identifier: MIT

#[cfg(unix)]
#[test]
#[ignore = "requires the pinned Gateway, MCP, and test-only Mod peer processes"]
fn actual_production_entrypoint_exact_restore_matrix_case()
-> Result<(), Box<dyn std::error::Error>> {
    #[allow(dead_code)]
    #[path = "../../../tests/support/completed_resume_process_support.rs"]
    mod process_support;

    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::Duration;

    let required = |name: &str| -> Result<String, Box<dyn std::error::Error>> {
        std::env::var(name)
            .map_err(|_| format!("{name} is required for this integration test").into())
    };
    let runtime_binary = required("STS2_EXACT_HARNESS_BINARY")?;
    let mcp_binary = required("STS2_EXACT_MCP_BINARY")?;
    let gateway_address = required("STS2_EXACT_GATEWAY_ADDR")?;
    let exact_store = std::path::PathBuf::from(required("STS2_EXACT_STORE")?);
    let lease_id = required("STS2_EXACT_LEASE_ID")?;
    let lease_epoch = required("STS2_EXACT_LEASE_EPOCH")?;
    let owner_json = required("STS2_EXACT_OWNER_JSON")?;
    let owner_json = {
        let owner: serde_json::Value = serde_json::from_str(&owner_json)?;
        let owner = owner.get("fence").cloned().unwrap_or(owner);
        serde_json::to_string(&owner)?
    };
    let root = fixture_workspace();
    let (branch_path, artifact_path, _checkpoint, _payload, compatibility, coverage) =
        publish_fixture(&root)?;
    let branch_store = SqliteBranchStore::open(&branch_path)?;
    let claim = branch_store
        .prepare_continuation_claim("experiment:exact-restore-fixture", "branch:selected")?;
    branch_store.snapshot_continuation_owner(&claim.operation_id, &owner_json)?;
    let bridge = root.join("deterministic-provider.py");
    let provider_log = root.join("provider-events.jsonl");
    let provider_log_json = serde_json::to_string(&provider_log.to_string_lossy())?;
    let bridge_source = format!(
        r#"#!/usr/bin/python3
import json
import sys

LOG = {provider_log_json}
for line in sys.stdin:
    request = json.loads(line)
    response = {{
        "wire_version": "sts2.exo-bridge-wire-v2",
        "request_id": "exact-restore-request",
        "turn_id": "exact-restore-turn",
        "outcome": "decision",
        "decision": {{
            "decision": "action",
            "action_id": "combat.end-turn",
            "rationale": "deterministic exact-restore continuation",
            "confidence": 90
        }},
        "error_code": None
    }}
    with open(LOG, "a", encoding="utf-8") as output:
        output.write(json.dumps({{"request": request, "response": response}},
                                sort_keys=True, separators=(",", ":")) + "\n")
    print(json.dumps(response, separators=(",", ":")), flush=True)
"#
    );
    std::fs::write(&bridge, bridge_source)?;
    std::fs::set_permissions(&bridge, std::fs::Permissions::from_mode(0o700))?;
    let mut command = Command::new(runtime_binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
        .env("STS2_GATEWAY_ADDR", gateway_address)
        .env("STS2_GATEWAY_TOKEN", required("STS2_EXACT_GATEWAY_TOKEN")?)
        .env(
            "STS2_RECOVERY_TOKEN",
            required("STS2_EXACT_RECOVERY_TOKEN")?,
        )
        .env(
            "STS2_RECOVERY_PRINCIPAL_ID",
            required("STS2_EXACT_RECOVERY_PRINCIPAL_ID")?,
        )
        .env(
            "STS2_RECOVERY_DEPLOYMENT_ID",
            required("STS2_EXACT_RECOVERY_DEPLOYMENT_ID")?,
        )
        .env(
            "STS2_RECOVERY_INSTANCE_ID",
            required("STS2_EXACT_RECOVERY_INSTANCE_ID")?,
        )
        .env(
            "STS2_RECOVERY_INSTANCE_INCAR",
            required("STS2_EXACT_RECOVERY_INSTANCE_INCAR")?,
        )
        .env(
            "STS2_RECOVERY_BOOT_ID",
            required("STS2_EXACT_RECOVERY_BOOT_ID")?,
        )
        .env(
            "STS2_RECOVERY_AUTHORITY_GENERATION",
            required("STS2_EXACT_RECOVERY_AUTHORITY_GENERATION")?,
        )
        .env(
            "STS2_RECOVERY_LEASE_ID",
            required("STS2_EXACT_RECOVERY_LEASE_ID")?,
        )
        .env(
            "STS2_RECOVERY_LEASE_EPOCH",
            required("STS2_EXACT_RECOVERY_LEASE_EPOCH")?,
        )
        .env(
            "STS2_RECOVERY_CURRENT_FENCE_JSON",
            required("STS2_EXACT_RECOVERY_CURRENT_FENCE_JSON")?,
        )
        .env("STS2_MCP_BINARY", mcp_binary)
        .env("STS2_INSTANCE_ID", required("STS2_EXACT_INSTANCE_ID")?)
        .env("STS2_CALLER_ID", required("STS2_EXACT_CALLER_ID")?)
        .env("STS2_SESSION_ID", required("STS2_EXACT_SESSION_ID")?)
        .env("STS2_LEASE_ID", lease_id)
        .env("STS2_LEASE_EPOCH", lease_epoch)
        .env("STS2_MCP_SESSION_ID", "exact-restore-mcp")
        .env("STS2_RUN_ID", "run:exact-restore")
        .env("STS2_EPISODE_ID", "episode:exact-restore")
        .env("STS2_TRAJECTORY_ID", "trajectory:exact-restore")
        .env("STS2_TRACE_ID", "trace:exact-restore")
        .env("STS2_ARTIFACT_ID", "artifact:exact-restore")
        .env("STS2_EXPERIMENT_ID", "experiment:exact-restore-fixture")
        .env("STS2_BRANCH_ID", "branch:selected")
        .env("STS2_BRANCH_STORE_PATH", &branch_path)
        .env(
            "STS2_EXECUTION_STORE_PATH",
            root.join("execution.sqlite3"),
        )
        .env("STS2_EXACT_ARTIFACT_STORE_PATH", &artifact_path)
        .env("STS2_EXACT_RESTORE_COMPATIBILITY_DIGEST", compatibility)
        .env("STS2_EXACT_RESTORE_COVERAGE_CONTRACT_DIGEST", coverage)
        .env("STS2_EXO_REVISION", sts2_harness::EXO_SOURCE_REVISION)
        .env("STS2_EXO_ADMISSION", "legacy")
        .env("STS2_EXO_BRIDGE_BINARY", &bridge)
        .env("STS2_EXO_BRIDGE_ARGS_JSON", "[]")
        .env("STS2_EXO_INHERITED_ENV_JSON", "[]")
        .env("STS2_EXO_REQUEST_ID", "exact-restore-request")
        .env("STS2_EXO_TURN_ID", "exact-restore-turn")
        .env("STS2_OBJECTIVE", "exact restore integration")
        .env("STS2_MAX_STEPS", "1");
    let output = process_support::run_child_with_timeout(command, Duration::from_secs(20))
        .map_err(|error| format!("production entrypoint failed: {error}"))?;
    if !output.status.success() {
        eprintln!(
            "exact-restore child exited {}; stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let branch = SqliteBranchStore::open(&branch_path)?
        .get("experiment:exact-restore-fixture", "branch:selected")?
        .ok_or("selected branch disappeared")?;
    match required("STS2_EXACT_EXPECTED_OUTCOME")?.as_str() {
        "positive" => {
            assert!(
                output.status.success(),
                "positive exact-restore child exited {}; stdout={}; stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(branch.status, DurableBranchStatus::Completed);
            assert_eq!(branch.assurance, BranchAssurance::ExactRestoreReceipt);
            assert!(branch
                .artifacts
                .iter()
                .any(|artifact| artifact.role == BranchArtifactRole::ContextSnapshot));
            let provider_events = std::fs::read_to_string(root.join("provider-events.jsonl"))?;
            assert!(
                provider_events.lines().count() >= 1,
                "positive continuation must produce a deterministic provider decision"
            );
            let effects: serde_json::Value = serde_json::from_slice(
                &std::fs::read(exact_store.join("runtime-v3-effects.json"))?,
            )?;
            assert_eq!(effects["action_count"], 1);
            assert_eq!(effects["settled_count"], 1);
            assert_eq!(effects["action_id"], "combat.end-turn");
        }
        "refused" => {
            assert!(
                !output.status.success(),
                "native-unsupported begin unexpectedly succeeded; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(branch.status, DurableBranchStatus::Failed);
            assert_ne!(branch.assurance, BranchAssurance::ExactRestoreReceipt);
            assert!(!branch
                .artifacts
                .iter()
                .any(|artifact| artifact.role == BranchArtifactRole::ContextSnapshot));
        }
        "unknown" => {
            assert!(
                !output.status.success(),
                "uncertain exact-restore child unexpectedly succeeded; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(branch.status, DurableBranchStatus::Unknown);
        }
        other => return Err(format!("unknown expected outcome {other}").into()),
    }
    let _post_restore_status = output.status;
    std::fs::remove_dir_all(root)?;
    Ok(())
}
