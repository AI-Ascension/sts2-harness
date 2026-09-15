// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BlobDigest, BranchArtifactReference, BranchArtifactRole, BranchArtifactState, BranchAssurance,
    BranchFork, BranchPruneRequest, BranchRetentionPolicy, BranchStoreError, BranchStrategy,
    DurableBranchDraft, DurableBranchStatus, ExactArtifactStore, ExactArtifactStoreResolver,
    ExactCheckpointId, ExactStateDigest, OccurrenceId, SqliteBranchStore,
};

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-branch-availability-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn state(value: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        value.to_string().repeat(64)
    ))
    .expect("valid state digest")
}

fn occurrence(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("valid occurrence")
}

fn artifact(id: &str, role: BranchArtifactRole) -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: id.to_owned(),
        role,
    }
}

fn draft(
    branch_id: &str,
    parent: Option<&str>,
    occurrence_id: &str,
    artifacts: Vec<BranchArtifactReference>,
) -> DurableBranchDraft {
    DurableBranchDraft {
        experiment_id: "experiment:durable".to_owned(),
        root_branch_id: "branch:root".to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(occurrence_id),
            parent_occurrence_id: parent.map(|_| occurrence("occurrence:root")),
            state_digest: state('a'),
        },
        strategy: BranchStrategy::ExactRestore,
        source_handle: Some(format!("source:{branch_id}")),
        trajectory_prefix: None,
        effective_seed: Some("seed:42".to_owned()),
        setup_digest: Some("setup:standard".to_owned()),
        boundary: "decision".to_owned(),
        assurance: BranchAssurance::Unverified,
        run_id: format!("run:{branch_id}"),
        episode_id: Some(format!("episode:{branch_id}")),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: "policy:v1".to_owned(),
        config_revision: "config:v1".to_owned(),
        name: branch_id.to_owned(),
        notes: Some("synthetic durable branch".to_owned()),
        artifacts,
    }
}

fn blob_path(root: &Path, digest: &BlobDigest) -> PathBuf {
    let hex = digest.as_str().trim_start_matches("sha256:");
    root.join("exact/blobs").join(&hex[..2]).join(hex)
}

fn manifest_path(root: &Path, identifier: &ExactCheckpointId) -> PathBuf {
    let hex = identifier
        .as_str()
        .trim_start_matches("asc-checkpoint:v1:sha256:");
    root.join("exact/manifests").join(&hex[..2]).join(hex)
}

#[test]
fn readable_manifest_and_blob_survive_branch_owner_restart() -> Result<(), BranchStoreError> {
    let root = workspace("restart");
    let store_path = root.join("branches.sqlite3");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let manifest = artifacts
        .publish_manifest(b"{}")
        .expect("manifest publishes");
    let blob = artifacts
        .stage_blob(b"exact-state-payload")
        .expect("blob stages");
    let references = vec![
        artifact(manifest.as_str(), BranchArtifactRole::Checkpoint),
        artifact(blob.as_str(), BranchArtifactRole::RestoreClosure),
    ];
    let store = SqliteBranchStore::open(&store_path)?;
    store.create(
        "operation:root",
        draft("branch:root", None, "occurrence:root", references.clone()),
    )?;
    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let availability =
        store.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert_eq!(availability.resolutions().len(), 2);
    assert!(availability.all_available(), "both references are readable");
    availability
        .require_all_available()
        .expect("readable references permit continuation");
    drop(store);

    let restarted = SqliteBranchStore::open(&store_path)?;
    let after = restarted.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert_eq!(
        after, availability,
        "restart preserves the recorded references and their resolved states"
    );
    assert_eq!(
        restarted
            .get("experiment:durable", "branch:root")?
            .expect("root branch")
            .artifacts,
        references
    );
    Ok(())
}

#[test]
fn absent_blob_reports_missing_and_is_never_silently_recreated() -> Result<(), BranchStoreError> {
    let root = workspace("absent");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let blob = artifacts.stage_blob(b"payload").expect("blob stages");
    let path = blob_path(artifacts.root_directory(), &blob);
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            vec![artifact(blob.as_str(), BranchArtifactRole::Checkpoint)],
        ),
    )?;
    fs::remove_file(&path).expect("stored blob is removed");

    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let availability =
        store.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert!(
        !availability.all_available(),
        "an absent blob is not readable"
    );
    assert_eq!(
        availability.unavailable().len(),
        1,
        "the absent blob is reported explicitly"
    );
    assert_eq!(
        availability.unavailable()[0].state,
        BranchArtifactState::Missing
    );
    let refusal = availability
        .require_all_available()
        .expect_err("an unreadable artifact refuses continuation");
    assert_eq!(refusal.references().len(), 1);
    assert_eq!(refusal.references()[0].state, BranchArtifactState::Missing);
    assert!(
        refusal.to_string().contains("missing"),
        "the refusal names the explicit state"
    );
    assert!(
        !path.exists(),
        "resolution must never recreate a missing artifact"
    );
    assert!(
        !store
            .artifact_availability("experiment:durable", "branch:root", &resolver)?
            .all_available(),
        "the unavailable state is stable rather than repaired by a later read"
    );
    Ok(())
}

#[test]
fn absent_manifest_reports_missing_without_recreating_the_checkpoint()
-> Result<(), BranchStoreError> {
    let root = workspace("absent-manifest");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let manifest = artifacts
        .publish_manifest(b"{}")
        .expect("manifest publishes");
    let path = manifest_path(artifacts.root_directory(), &manifest);
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            vec![artifact(manifest.as_str(), BranchArtifactRole::Checkpoint)],
        ),
    )?;
    fs::remove_file(&path).expect("stored manifest is removed");

    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let availability =
        store.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert_eq!(
        availability.unavailable()[0].state,
        BranchArtifactState::Missing,
        "an expired checkpoint manifest is reported, not re-derived"
    );
    assert!(
        !path.exists(),
        "a missing checkpoint manifest is never silently recreated"
    );
    Ok(())
}

#[test]
fn tampered_blob_bytes_report_unverifiable_rather_than_missing() -> Result<(), BranchStoreError> {
    let root = workspace("tampered");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let blob = artifacts.stage_blob(b"payload").expect("blob stages");
    let path = blob_path(artifacts.root_directory(), &blob);
    fs::write(&path, b"substituted-bytes").expect("stored blob is substituted");
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            vec![artifact(blob.as_str(), BranchArtifactRole::Checkpoint)],
        ),
    )?;
    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let availability =
        store.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert_eq!(
        availability.unavailable()[0].state,
        BranchArtifactState::Unverifiable,
        "bytes that no longer match their identity are unverifiable, not absent"
    );
    Ok(())
}

#[test]
fn identity_outside_the_verified_namespaces_is_unverifiable() -> Result<(), BranchStoreError> {
    let root = workspace("namespace");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            vec![
                artifact("artifact:opaque", BranchArtifactRole::Checkpoint),
                artifact("sha256:not-hex", BranchArtifactRole::RestoreClosure),
            ],
        ),
    )?;
    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let availability =
        store.artifact_availability("experiment:durable", "branch:root", &resolver)?;
    assert_eq!(availability.resolutions().len(), 2);
    assert!(
        availability
            .resolutions()
            .iter()
            .all(|resolution| resolution.state == BranchArtifactState::Unverifiable),
        "an identity the store cannot vouch for never reports available"
    );
    Ok(())
}

#[test]
fn pruning_one_sibling_keeps_the_shared_artifact_readable_for_the_other()
-> Result<(), BranchStoreError> {
    let root = workspace("prune");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let shared = artifacts
        .stage_blob(b"shared-payload")
        .expect("shared stages");
    let left_only = artifacts
        .stage_blob(b"left-only-payload")
        .expect("left stages");
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    let shared_reference = artifact(shared.as_str(), BranchArtifactRole::Checkpoint);
    let left_reference = artifact(left_only.as_str(), BranchArtifactRole::ReplayPrefix);
    store.create(
        "operation:root",
        draft("branch:root", None, "occurrence:root", Vec::new()),
    )?;
    store.create(
        "operation:left",
        draft(
            "branch:left",
            Some("branch:root"),
            "occurrence:left",
            vec![shared_reference.clone(), left_reference.clone()],
        ),
    )?;
    store.create(
        "operation:right",
        draft(
            "branch:right",
            Some("branch:root"),
            "occurrence:right",
            vec![shared_reference.clone()],
        ),
    )?;
    store.transition(
        "operation:left-archive",
        "experiment:durable",
        "branch:left",
        0,
        DurableBranchStatus::Archived,
    )?;
    let request = BranchPruneRequest {
        experiment_id: "experiment:durable".to_owned(),
        branch_ids: vec!["branch:left".to_owned()],
        policy: BranchRetentionPolicy::default(),
    };
    let plan = store.prune("operation:prune-left", &request)?;
    assert_eq!(plan.collectable_artifacts, vec![left_reference]);
    assert_eq!(
        plan.retained_artifacts,
        vec![shared_reference],
        "the shared blob stays pinned by the surviving sibling"
    );
    assert!(
        blob_path(artifacts.root_directory(), &shared).exists(),
        "pruning metadata does not collect a blob another branch still needs"
    );

    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    let right = store.artifact_availability("experiment:durable", "branch:right", &resolver)?;
    assert!(
        right.all_available(),
        "the surviving sibling keeps reading the shared artifact"
    );
    assert_eq!(
        store
            .artifact_availability("experiment:durable", "branch:left", &resolver)
            .expect_err("a pruned branch never reports a vacuously available remainder"),
        BranchStoreError::ArtifactUnavailable
    );
    Ok(())
}

#[test]
fn unknown_branch_and_invalid_labels_fail_closed() -> Result<(), BranchStoreError> {
    let root = workspace("fail-closed");
    let artifacts = ExactArtifactStore::new(root.join("exact-store"));
    let store = SqliteBranchStore::open(root.join("branches.sqlite3"))?;
    store.create(
        "operation:root",
        draft("branch:root", None, "occurrence:root", Vec::new()),
    )?;
    let resolver = ExactArtifactStoreResolver::new(&artifacts);
    assert_eq!(
        store
            .artifact_availability("experiment:durable", "branch:absent", &resolver)
            .expect_err("an unknown branch is refused"),
        BranchStoreError::UnknownBranch
    );
    assert_eq!(
        store
            .artifact_availability("", "branch:root", &resolver)
            .expect_err("an empty experiment label is refused"),
        BranchStoreError::InvalidInput
    );
    assert_eq!(
        store
            .artifact_availability("experiment:durable", "branch:root", &resolver)?
            .resolutions()
            .len(),
        0,
        "a branch that retains nothing resolves to an empty ordered set"
    );
    Ok(())
}
