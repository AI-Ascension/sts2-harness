// SPDX-License-Identifier: MIT

use super::*;

use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchContinuationSelector,
    BranchFork, BranchStrategy, DurableBranchDraft, DurableBranchStatus, ExactArtifactStore,
    ExactStateDigest, OccurrenceId, SqliteBranchStore,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn profile_pins_require_both_independent_sha256_values() {
    assert!(
        validate_pin(
            "pin",
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )
        .is_ok()
    );
    assert!(validate_pin("pin", "sha256:ABCDEF").is_err());
    assert!(validate_pin("pin", "not-a-digest").is_err());
}

#[test]
fn base64_chunks_preserve_empty_and_padding_cases() {
    assert_eq!(encode_base64(b""), "");
    assert_eq!(encode_base64(b"f"), "Zg==");
    assert_eq!(encode_base64(b"fo"), "Zm8=");
    assert_eq!(encode_base64(b"foo"), "Zm9v");
}

#[test]
fn wrapper_binds_the_exact_inner_message_identity() -> Result<(), Box<dyn std::error::Error>> {
    let frame = json!({
        "contract": NEUTRAL_CONTRACT,
        "schema_digest": NEUTRAL_SCHEMA_DIGEST,
        "message_id": "00000000-0000-4000-8000-000000000001",
        "correlation_id": "00000000-0000-4000-8000-000000000002",
        "kind": "exact_restore_lookup_request",
        "payload": {
            "operation_id": "00000000-0000-4000-8000-000000000003",
            "expected_owner": {
                "instance_id": "instance-1",
                "lease_id": "lease-1",
                "lease_epoch": 1,
                "generation": 1,
                "owner_id": "owner-1"
            }
        }
    });
    let wrapper = wrapper_request(&frame, "harness-principal")?;
    let validator = wrapper_validator()?;
    assert!(valid_schema(validator, &wrapper));
    assert_eq!(wrapper["message_id"], frame["message_id"]);
    assert_eq!(wrapper["correlation_id"], frame["correlation_id"]);
    assert_eq!(wrapper["payload"]["frame"], frame);
    Ok(())
}

fn synthetic_state(payload: &[u8]) -> Result<ExactStateDigest, Box<dyn std::error::Error>> {
    let mut hash = Sha256::new();
    hash.update(b"AI-ASCENSION/EXACT-STATE/v1\0");
    hash.update(payload);
    let digest = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{digest}"
    ))?)
}

fn synthetic_draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    state: &ExactStateDigest,
    artifacts: Vec<BranchArtifactReference>,
) -> Result<DurableBranchDraft, Box<dyn std::error::Error>> {
    let occurrence_id = OccurrenceId::parse(&format!("occurrence:{branch_id}"))
        .map_err(|error| format!("occurrence id: {error:?}"))?;
    let parent_occurrence_id = parent_branch_id
        .map(|_| OccurrenceId::parse("occurrence:branch:root"))
        .transpose()
        .map_err(|error| format!("parent occurrence id: {error:?}"))?;
    Ok(DurableBranchDraft {
        experiment_id: String::from("experiment:exact-restore-fixture"),
        root_branch_id: String::from("branch:root"),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id,
            parent_occurrence_id,
            state_digest: state.clone(),
        },
        strategy: BranchStrategy::ExactRestore,
        source_handle: Some(String::from("synthetic-checkpoint")),
        trajectory_prefix: None,
        effective_seed: Some(String::from("seed:fixture")),
        setup_digest: Some(String::from("setup:fixture")),
        boundary: String::from("decision"),
        assurance: BranchAssurance::Unverified,
        run_id: format!("run:{branch_id}"),
        episode_id: Some(String::from("episode:exact-restore-fixture")),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(String::from("context:fixture")),
        policy_revision: String::from("policy:fixture"),
        config_revision: String::from("config:fixture"),
        name: branch_id.to_owned(),
        notes: Some(String::from("synthetic source-only fixture")),
        artifacts,
    })
}

fn fixture_workspace() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-exact-restore-fixture-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

struct PublishedFixture {
    branch_path: PathBuf,
    artifact_path: PathBuf,
    checkpoint: String,
    payload: String,
    compatibility: String,
    coverage: String,
}

fn publish_fixture(root: &Path) -> Result<PublishedFixture, Box<dyn std::error::Error>> {
    let artifact_path = root.join("artifacts");
    let branch_path = root.join("branches.sqlite3");
    let store = ExactArtifactStore::new(&artifact_path);
    let canonical_source: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../protocol-artifact/exact-state-v1/golden/state.canonical"
    ))?;
    let mut canonical_value = canonical_source;
    canonical_value["boundary"]["game_tick"] = serde_json::json!(7);
    let canonical = serde_json::to_vec(&canonical_value)?;
    let canonical_digest = store.stage_blob(&canonical)?.as_str().to_owned();
    let state = synthetic_state(&canonical)?;
    let compatibility = format!(
        "sha256:{}",
        Sha256::digest(serde_json::to_vec(&canonical_value["compatibility"])?.as_slice())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let coverage = canonical_value["compatibility"]["coverage_contract_digest"]
        .as_str()
        .ok_or("canonical payload omitted coverage contract")?
        .to_owned();
    let manifest = serde_json::to_vec(&json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "canonical_profile": "asc-jcs-state-v1",
        "boundary": canonical_value["boundary"],
        "origin": {"run_id":"run:branch:selected", "generation":7},
        "parent_checkpoint_id": null,
        "exact_state_digest": state.as_str(),
        "canonical_payload": {
            "digest": canonical_digest,
            "role":"exact_state_payload",
            "codec":"asc-jcs-state-v1",
            "size_bytes":canonical.len()
        },
        "restore_artifacts": [{
            "digest": canonical_digest,
            "role":"snapshot",
            "codec":"test-raw-v1",
            "size_bytes":canonical.len()
        }],
        "compatibility_digest": compatibility.clone(),
        "coverage_contract_digest": coverage.clone()
    }))?;
    let checkpoint = store.publish_manifest(&manifest)?;
    let root_record = SqliteBranchStore::open(&branch_path)?.create(
        "operation:create-root",
        synthetic_draft("branch:root", None, &state, Vec::new())?,
    )?;
    let branch_store = SqliteBranchStore::open(&branch_path)?;
    let child = branch_store.create(
        "operation:create-child",
        synthetic_draft(
            "branch:selected",
            Some("branch:root"),
            &state,
            vec![
                BranchArtifactReference {
                    artifact_id: checkpoint.as_str().to_owned(),
                    role: BranchArtifactRole::Checkpoint,
                },
                BranchArtifactReference {
                    artifact_id: canonical_digest.clone(),
                    role: BranchArtifactRole::RestoreClosure,
                },
            ],
        )?,
    )?;
    let restoring = branch_store.transition(
        "operation:restore",
        "experiment:exact-restore-fixture",
        "branch:selected",
        child.metadata_revision,
        DurableBranchStatus::Restoring,
    )?;
    let assured = branch_store.set_assurance(
        "operation:receipt",
        "experiment:exact-restore-fixture",
        "branch:selected",
        restoring.metadata_revision,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    branch_store.transition(
        "operation:ready",
        "experiment:exact-restore-fixture",
        "branch:selected",
        assured.metadata_revision,
        DurableBranchStatus::Ready,
    )?;
    drop(root_record);
    Ok(PublishedFixture {
        branch_path,
        artifact_path,
        checkpoint: checkpoint.as_str().to_owned(),
        payload: canonical_digest,
        compatibility,
        coverage,
    })
}

#[test]
fn valid_manifest_and_selected_branch_form_one_deduplicated_restore_closure()
-> Result<(), Box<dyn std::error::Error>> {
    let root = fixture_workspace()?;
    let PublishedFixture {
        branch_path,
        artifact_path,
        checkpoint,
        payload,
        compatibility,
        coverage,
    } = publish_fixture(&root)?;
    let selector =
        BranchContinuationSelector::new("experiment:exact-restore-fixture", "branch:selected")?;
    let selected =
        crate::runtime_support::branch_continuation_runtime::SelectedBranchContinuation::load(
            &selector,
            &branch_path,
            &artifact_path,
        )?;
    let closure = VerifiedClosure::prepare(
        &selected,
        &artifact_path,
        ProfilePins {
            compatibility_digest: compatibility,
            coverage_contract_digest: coverage,
        },
    )?;
    assert_eq!(closure.checkpoint_id, checkpoint);
    assert_eq!(closure.distinct_blob_count, 1);
    assert_eq!(closure.transfer_blobs.len(), 2);
    assert_eq!(closure.transfer_blobs[1].digest, payload);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn actual_mcp_rejection_stops_before_any_chunk_upload() -> Result<(), Box<dyn std::error::Error>> {
    let temporary =
        std::env::temp_dir().join(format!("sts2-exact-restore-mcp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temporary)?;
    let script = temporary.join("mcp.py");
    let log = temporary.join("calls.log");
    let log_json = serde_json::to_string(&log.to_string_lossy())?;
    let source = format!(
        r#"#!/usr/bin/python3
import hashlib, json, sys
LOG = {log_json}
TOOLS = ["sts2.exact_restore.begin", "sts2.exact_restore.put_chunk",
         "sts2.exact_restore.finish_blob", "sts2.exact_restore.commit",
         "sts2.exact_restore.lookup"]
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    if method == "initialize":
        result = {{}}
    elif method == "tools/list":
        result = {{"revision":"exact-restore-v1-mcp",
                  "tools":[{{"name":name}} for name in TOOLS]}}
    elif method == "tools/call":
        name = request["params"]["name"]
        with open(LOG, "a", encoding="utf-8") as output:
            output.write(name + "\n")
        wrapper = request["params"]["arguments"]
        inner = wrapper["payload"]["frame"]
        canonical = json.dumps(inner, sort_keys=True, separators=(",", ":")).encode("utf-8")
        frame = {{
            "contract":"sts2-exact-restore-v1",
            "schema_digest":"2289d888c33eac46873408303c4423eab762e3f7bd6132ae8ae88d0d3b1858e4",
            "message_id":"00000000-0000-4000-8000-000000000010",
            "correlation_id":inner["message_id"],
            "kind":"exact_restore_error_response",
            "payload":{{
                "operation_id":inner["payload"]["operation_id"],
                "expected_owner":inner["payload"]["expected_owner"],
                "request_digest":"sha256:" + hashlib.sha256(canonical).hexdigest(),
                "outcome":"REJECTED",
                "error_code":"no_restore_adapter",
                "host_effect":"not_started"
            }}
        }}
        # MCP exact-restore advertises a Gateway wrapper on input but projects
        # the validated neutral frame as the tools/call content on output.
        result = {{"content":[{{"type":"text","text":json.dumps(frame, sort_keys=True, separators=(",", ":"))}}]}}
    else:
        raise RuntimeError("unexpected MCP method")
    print(json.dumps({{"jsonrpc":"2.0","id":request["id"],"result":result}},
                     separators=(",", ":")), flush=True)
"#
    );
    std::fs::write(&script, source)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let config = crate::runtime_support::config::RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("token"),
        mcp_binary: script.to_string_lossy().into_owned(),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: vec![(String::from("STS2_RECOVERY_TOKEN"), String::from("token"))],
    };
    let owner = json!({
        "deployment_id":"00000000-0000-4000-8000-000000000001",
        "instance_id":"00000000-0000-4000-8000-000000000002",
        "instance_incarnation":"00000000-0000-4000-8000-000000000003",
        "boot_id":"00000000-0000-4000-8000-000000000004",
        "authority_generation":1,
        "host_fence_id":"00000000-0000-4000-8000-000000000005",
        "host_fence_generation":1,
        "lease_id":"00000000-0000-4000-8000-000000000006",
        "lease_epoch":1,
        "session_id":"session-1",
        "lease_expires_at_millis":1_900_000_000_000_u64
    });
    let request = operation::request_frame(
        "exact_restore_lookup_request",
        json!({
            "operation_id":"00000000-0000-4000-8000-000000000007",
            "expected_owner":owner,
        }),
    )?;
    let mut process = None;
    let mut rpc_id = 3;
    let response = operation::exchange(
        &mut process,
        &mut rpc_id,
        &config,
        &request,
        "sts2.exact_restore.begin",
    )
    .map_err(|error| error.message)?;
    let close = process.as_mut().map_or(Ok(()), |mcp| mcp.close());
    assert!(close.is_ok());
    assert_eq!(response["kind"], "exact_restore_error_response");
    assert_eq!(response["payload"]["host_effect"], "not_started");
    assert_eq!(
        std::fs::read_to_string(&log)?.lines().collect::<Vec<_>>(),
        ["sts2.exact_restore.begin"]
    );
    std::fs::remove_dir_all(temporary)?;
    Ok(())
}

include!("exact_restore_actual_entry_tests.rs");
