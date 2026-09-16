// SPDX-License-Identifier: MIT

//! Production-entry regression for a selected Running prefix branch.
//!
//! The real harness runtime binary must adopt the retained owner before opening MCP, must never
//! call the fresh allocation route, and must withhold the provider until the first observation
//! matches the persisted checkpoint boundary.

#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchContinuationClaimState,
    BranchFork, BranchStrategy, CatalogEvidence, Checkpoint, DurableBranchDraft,
    DurableBranchStatus, EXO_SOURCE_REVISION, ExactArtifactStore, ExactStateDigest,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, OccurrenceId,
    OperationIntent, OperationState, SqliteBranchStore,
};

#[path = "support/completed_resume_process_support.rs"]
#[allow(dead_code)]
mod process_support;

#[path = "support/branch_continuation_running_resume_case.rs"]
mod process_case;

#[path = "support/branch_continuation_running_resume_durable_fixture.rs"]
mod durable_fixture;

const EXPERIMENT_ID: &str = "experiment:running-resume-process";
const ROOT_BRANCH_ID: &str = "branch:root";
const BRANCH_ID: &str = "branch:selected";
const RUN_ID: &str = "run:branch:selected";
const EPISODE_ID: &str = "episode:branch:selected";
const ATTEMPT_ID: &str = "attempt:branch:selected";
const TRAJECTORY_ID: &str = "trajectory:branch:selected";
const INSTANCE_ID: &str = "00000000-0000-4000-8000-000000000002";
const CALLER_ID: &str = "selected-principal";
const SESSION_ID: &str = "selected-session";
const MCP_SESSION_ID: &str = "selected-mcp-session";
const DEPLOYMENT_ID: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE_INCAR: &str = "00000000-0000-4000-8000-000000000003";
const BOOT_ID: &str = "00000000-0000-4000-8000-000000000004";
const FENCE_ID: &str = "00000000-0000-4000-8000-000000000005";
const LEASE_ID: &str = "00000000-0000-4000-8000-000000000006";
const OWNER_ADOPT_SCHEMA: &str = "7240e2f5054e5f6639ad12fa1ed66d40992f1234e49b693b2aacc53451dec380";
const ALLOCATION_SCHEMA: &str = "ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sts2-running-branch-resume-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn state_digest() -> Result<ExactStateDigest, Box<dyn std::error::Error>> {
    let value = format!("asc-state:v1:sha256:{}", "b".repeat(64));
    ExactStateDigest::parse(&value).map_err(|error| error.to_string().into())
}

fn branch_draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    strategy: BranchStrategy,
    artifact: Option<BranchArtifactReference>,
    effective_seed: &str,
) -> Result<DurableBranchDraft, Box<dyn std::error::Error>> {
    let exact = strategy == BranchStrategy::ExactRestore;
    let occurrence_text = format!("occurrence:{branch_id}");
    let occurrence_id =
        OccurrenceId::parse(&occurrence_text).map_err(|_| "invalid fixture occurrence")?;
    let parent_occurrence_id = if parent_branch_id.is_some() {
        Some(
            OccurrenceId::parse("occurrence:branch:root")
                .map_err(|_| "invalid fixture parent occurrence")?,
        )
    } else {
        None
    };
    Ok(DurableBranchDraft {
        experiment_id: EXPERIMENT_ID.to_owned(),
        root_branch_id: ROOT_BRANCH_ID.to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id,
            parent_occurrence_id,
            state_digest: state_digest()?,
        },
        strategy,
        source_handle: exact.then(|| format!("checkpoint:{branch_id}")),
        trajectory_prefix: (!exact).then(|| format!("trajectory-prefix:{branch_id}")),
        effective_seed: Some(effective_seed.to_owned()),
        setup_digest: Some(String::from("setup:selected")),
        boundary: String::from("observation"),
        assurance: BranchAssurance::Unverified,
        run_id: if branch_id == BRANCH_ID {
            RUN_ID.to_owned()
        } else {
            format!("run:{branch_id}")
        },
        episode_id: Some(if branch_id == BRANCH_ID {
            EPISODE_ID.to_owned()
        } else {
            format!("episode:{branch_id}")
        }),
        trajectory_id: Some(if branch_id == BRANCH_ID {
            TRAJECTORY_ID.to_owned()
        } else {
            format!("trajectory:{branch_id}")
        }),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: String::from("policy:resume-process"),
        config_revision: String::from("config:resume-process"),
        name: branch_id.to_owned(),
        notes: None,
        artifacts: artifact.into_iter().collect(),
    })
}

fn owner(expiry: u64) -> Value {
    json!({
        "deployment_id": DEPLOYMENT_ID,
        "instance_id": INSTANCE_ID,
        "instance_incarnation": INSTANCE_INCAR,
        "boot_id": BOOT_ID,
        "authority_generation": 7,
        "host_fence_id": FENCE_ID,
        "host_fence_generation": 3,
        "lease_id": LEASE_ID,
        "lease_epoch": 8,
        "session_id": SESSION_ID,
        "lease_expires_at_millis": expiry
    })
}

fn seed_branch_and_boundary(
    root: &Path,
    branch_seed: &str,
    pending_unknown: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let branch_path = root.join("branches.sqlite3");
    let artifact_path = root.join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);

    let root_blob = artifacts.stage_blob(b"source checkpoint")?;
    let child_blob = artifacts.stage_blob(b"retained public prefix")?;
    let store = SqliteBranchStore::open(&branch_path)?;
    let root_branch = store.create(
        "operation:create-root",
        branch_draft(
            ROOT_BRANCH_ID,
            None,
            BranchStrategy::ExactRestore,
            Some(BranchArtifactReference {
                artifact_id: root_blob.as_str().to_owned(),
                role: BranchArtifactRole::Checkpoint,
            }),
            "seed:selected",
        )?,
    )?;
    let root_branch = store.transition(
        "operation:root-restoring",
        EXPERIMENT_ID,
        ROOT_BRANCH_ID,
        root_branch.metadata_revision,
        DurableBranchStatus::Restoring,
    )?;
    let root_branch = store.set_assurance(
        "operation:root-assurance",
        EXPERIMENT_ID,
        ROOT_BRANCH_ID,
        root_branch.metadata_revision,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    store.transition(
        "operation:root-ready",
        EXPERIMENT_ID,
        ROOT_BRANCH_ID,
        root_branch.metadata_revision,
        DurableBranchStatus::Ready,
    )?;

    let child = store.create(
        "operation:create-child",
        branch_draft(
            BRANCH_ID,
            Some(ROOT_BRANCH_ID),
            BranchStrategy::PrefixReplay,
            Some(BranchArtifactReference {
                artifact_id: child_blob.as_str().to_owned(),
                role: BranchArtifactRole::ReplayPrefix,
            }),
            branch_seed,
        )?,
    )?;
    let child = store.transition(
        "operation:child-replaying",
        EXPERIMENT_ID,
        BRANCH_ID,
        child.metadata_revision,
        DurableBranchStatus::Replaying,
    )?;
    let child = store.set_assurance(
        "operation:child-boundary",
        EXPERIMENT_ID,
        BRANCH_ID,
        child.metadata_revision,
        BranchAssurance::PrefixReplayBoundary,
    )?;
    let child = store.transition(
        "operation:child-ready",
        EXPERIMENT_ID,
        BRANCH_ID,
        child.metadata_revision,
        DurableBranchStatus::Ready,
    )?;
    store.transition(
        "operation:child-running",
        EXPERIMENT_ID,
        BRANCH_ID,
        child.metadata_revision,
        DurableBranchStatus::Running,
    )?;

    let claim = store.prepare_continuation_claim(EXPERIMENT_ID, BRANCH_ID)?;
    let expiry = now_millis()?.saturating_add(3_600_000);
    let owner = owner(expiry);
    let claim =
        store.snapshot_continuation_owner(&claim.operation_id, &serde_json::to_string(&owner)?)?;
    let claim = store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::OwnerSnapshotted,
        BranchContinuationClaimState::Claimed,
    )?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::Claimed,
        BranchContinuationClaimState::BoundaryVerified,
    )?;
    drop(store);

    durable_fixture::seed_execution_boundary(root, pending_unknown)?;
    Ok(())
}

fn now_millis() -> Result<u64, Box<dyn std::error::Error>> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64)
}

fn recovery_authority(owner: &Value) -> Value {
    json!({
        "contract":"watchdog-runtime-allocation-v1",
        "schema_digest":ALLOCATION_SCHEMA,
        "context":{
            "deployment_id":owner["deployment_id"],
            "instance_id":owner["instance_id"],
            "instance_incarnation":owner["instance_incarnation"],
            "boot_id":owner["boot_id"],
            "authority_generation":owner["authority_generation"],
            "lease_id":owner["lease_id"],
            "lease_epoch":owner["lease_epoch"]
        },
        "current_fence":{
            "host_fence_id":owner["host_fence_id"],
            "deployment_id":owner["deployment_id"],
            "instance_id":owner["instance_id"],
            "instance_incarnation":owner["instance_incarnation"],
            "boot_id":owner["boot_id"],
            "authority_generation":owner["authority_generation"],
            "fence_generation":owner["host_fence_generation"],
            "created_at":"2026-09-16T00:00:00Z"
        }
    })
}

#[test]
fn running_prefix_resume_adopts_before_observation_and_gates_provider_on_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let matching = process_case::run_case(false, false, false)?;
    assert!(
        matching.provider_called,
        "matching boundary never reached provider: {}; routes={:?}; MCP={}",
        matching.output, matching.gateway_paths, matching.mcp_log
    );
    assert_eq!(
        matching.gateway_paths.first().map(String::as_str),
        Some("/v1/recovery/continuation/owner/adopt"),
        "runtime did not adopt the existing owner first: {:?}",
        matching.gateway_paths
    );
    assert!(
        !matching
            .gateway_paths
            .iter()
            .any(|path| path == "/v1/sessions/allocate"),
        "selected Running resume attempted fresh allocation: {:?}",
        matching.gateway_paths
    );
    assert!(
        matching.mcp_log.contains("tools/call sts2.observe"),
        "runtime did not observe the selected live destination: {}",
        matching.mcp_log
    );
    assert!(
        !matching.mcp_log.contains("start_seeded_run"),
        "selected Running resume attempted a seeded start: {}",
        matching.mcp_log
    );

    let mismatch = process_case::run_case(true, false, false)?;
    assert!(
        !mismatch.provider_called,
        "provider ran before mismatched observation refusal: {}",
        mismatch.output
    );
    assert_eq!(
        mismatch.gateway_paths.first().map(String::as_str),
        Some("/v1/recovery/continuation/owner/adopt"),
        "mismatched resume did not adopt the current owner first: {:?}",
        mismatch.gateway_paths
    );
    assert!(
        !mismatch
            .gateway_paths
            .iter()
            .any(|path| path == "/v1/sessions/allocate"),
        "mismatched Running resume attempted fresh allocation: {:?}",
        mismatch.gateway_paths
    );
    assert!(
        mismatch.mcp_log.contains("tools/call sts2.observe"),
        "mismatched scenario did not reach authoritative observation: {}",
        mismatch.mcp_log
    );
    assert!(
        !mismatch
            .gateway_paths
            .iter()
            .any(|path| path.ends_with("/release")),
        "mismatched boundary released a lease it no longer owns: {:?}",
        mismatch.gateway_paths
    );

    let seed_mismatch = process_case::run_case(false, true, false)?;
    assert!(
        !seed_mismatch.provider_called,
        "seed mismatch reached the provider: {}",
        seed_mismatch.output
    );
    assert!(
        seed_mismatch.gateway_paths.is_empty(),
        "seed mismatch contacted Gateway: {:?}",
        seed_mismatch.gateway_paths
    );
    assert!(
        seed_mismatch.mcp_log.is_empty(),
        "seed mismatch launched MCP: {}",
        seed_mismatch.mcp_log
    );
    assert!(
        seed_mismatch.output.contains(
            "selected branch effective seed does not match its durable execution fingerprint"
        ),
        "seed mismatch was not rejected against the durable fingerprint: {}",
        seed_mismatch.output
    );

    let pending = process_case::run_case(false, false, true)?;
    assert!(
        !pending.provider_called,
        "provider ran while an operation remained unresolved: {}",
        pending.output
    );
    assert!(
        pending.gateway_paths.is_empty(),
        "resume contacted Gateway before refusing pending operation: {:?}",
        pending.gateway_paths
    );
    assert!(
        pending.mcp_log.is_empty(),
        "resume contacted MCP before refusing pending operation: {}",
        pending.mcp_log
    );
    assert!(
        pending
            .output
            .contains("selected-branch resume is blocked by unresolved durable operation"),
        "pending operation was not refused with fail-closed reason: {}",
        pending.output
    );
    assert!(
        pending.unknown_operation_retained,
        "unknown operation was not retained for authoritative reconciliation"
    );
    Ok(())
}
