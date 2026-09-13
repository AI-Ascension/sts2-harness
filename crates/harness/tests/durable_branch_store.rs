// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::PathBuf;

use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchFork, BranchPruneRequest,
    BranchRetentionPolicy, BranchStoreError, BranchStrategy, DurableBranchDraft,
    DurableBranchStatus, ExactStateDigest, OccurrenceId, SqliteBranchStore,
};

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

fn draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    occurrence_id: &str,
    parent_occurrence_id: Option<&str>,
    strategy: BranchStrategy,
) -> DurableBranchDraft {
    DurableBranchDraft {
        experiment_id: "experiment:durable".to_owned(),
        root_branch_id: "branch:root".to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(occurrence_id),
            parent_occurrence_id: parent_occurrence_id.map(occurrence),
            state_digest: state('a'),
        },
        strategy,
        source_handle: Some(format!("source:{branch_id}")),
        trajectory_prefix: (strategy == BranchStrategy::PrefixReplay)
            .then(|| format!("trajectory:{branch_id}")),
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
        artifacts: Vec::new(),
    }
}

fn artifact(id: &str, role: BranchArtifactRole) -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: id.to_owned(),
        role,
    }
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sts2-durable-branch-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ))
}

#[test]
fn restart_preserves_siblings_edges_and_equal_state_identity() -> Result<(), BranchStoreError> {
    let path = temp_path("restart");
    let _ = std::fs::remove_file(&path);
    let store = SqliteBranchStore::open(&path)?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.create(
        "operation:left",
        draft(
            "branch:left",
            Some("branch:root"),
            "occurrence:left",
            Some("occurrence:root"),
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.create(
        "operation:right",
        draft(
            "branch:right",
            Some("branch:root"),
            "occurrence:right",
            Some("occurrence:root"),
            BranchStrategy::PrefixReplay,
        ),
    )?;
    drop(store);

    let reopened = SqliteBranchStore::open(&path)?;
    let page = reopened.list("experiment:durable", None, 2)?;
    assert_eq!(page.branches.len(), 2);
    assert_eq!(page.next_cursor.as_deref(), Some("branch:root"));
    let tail = reopened.list("experiment:durable", page.next_cursor.as_deref(), 2)?;
    assert_eq!(tail.branches.len(), 0);
    let ancestry = reopened.ancestry("experiment:durable", "branch:left")?;
    assert_eq!(
        ancestry
            .iter()
            .map(|branch| branch.branch_id.as_str())
            .collect::<Vec<_>>(),
        ["branch:root", "branch:left"]
    );
    assert_eq!(
        reopened
            .get("experiment:durable", "branch:left")?
            .map(|branch| branch.fork.state_digest.clone()),
        Some(state('a'))
    );
    assert_eq!(
        reopened
            .get("experiment:durable", "branch:left")?
            .map(|branch| branch.fork.occurrence_id.clone()),
        Some(occurrence("occurrence:left"))
    );
    std::fs::remove_file(path).map_err(|error| BranchStoreError::Persistence(error.to_string()))?;
    Ok(())
}

#[test]
fn create_retry_is_idempotent_but_conflicting_and_invalid_parents_fail()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    let root = draft(
        "branch:root",
        None,
        "occurrence:root",
        None,
        BranchStrategy::ExactRestore,
    );
    let first = store.create("operation:root", root.clone())?;
    let retry = store.create("operation:root", root)?;
    assert_eq!(first, retry);
    assert_eq!(
        store
            .create(
                "operation:root",
                draft(
                    "branch:other",
                    Some("branch:root"),
                    "occurrence:other",
                    Some("occurrence:root"),
                    BranchStrategy::ExactRestore,
                ),
            )
            .expect_err("conflicting operation"),
        BranchStoreError::IdempotencyConflict
    );
    assert_eq!(
        store
            .create(
                "operation:missing",
                draft(
                    "branch:child",
                    Some("branch:missing"),
                    "occurrence:child",
                    Some("occurrence:root"),
                    BranchStrategy::ExactRestore,
                ),
            )
            .expect_err("unknown parent"),
        BranchStoreError::UnknownParent
    );
    Ok(())
}

#[test]
fn readiness_requires_strategy_specific_assurance_and_metadata_is_cas_bound()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    assert_eq!(
        store
            .transition(
                "operation:restore",
                "experiment:durable",
                "branch:root",
                0,
                DurableBranchStatus::Restoring,
            )?
            .status,
        DurableBranchStatus::Restoring
    );
    assert_eq!(
        store
            .transition(
                "operation:ready-before-proof",
                "experiment:durable",
                "branch:root",
                1,
                DurableBranchStatus::Ready,
            )
            .expect_err("unverified restore cannot be ready"),
        BranchStoreError::InsufficientAssurance
    );
    let assured = store.set_assurance(
        "operation:assurance",
        "experiment:durable",
        "branch:root",
        1,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    assert_eq!(assured.metadata_revision, 2);
    let ready = store.transition(
        "operation:ready",
        "experiment:durable",
        "branch:root",
        2,
        DurableBranchStatus::Ready,
    )?;
    assert_eq!(ready.status, DurableBranchStatus::Ready);
    let renamed = store.rename(
        "operation:rename",
        "experiment:durable",
        "branch:root",
        3,
        "root-renamed",
    )?;
    assert_eq!(renamed.name, "root-renamed");
    assert_eq!(
        store
            .rename(
                "operation:stale",
                "experiment:durable",
                "branch:root",
                3,
                "stale",
            )
            .expect_err("stale metadata revision"),
        BranchStoreError::StaleRevision
    );
    Ok(())
}

#[test]
fn artifact_prune_keeps_shared_dependencies_and_tombstones_collectable_edges()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    for (operation, branch, occurrence_id) in [
        ("operation:left", "branch:left", "occurrence:left"),
        ("operation:right", "branch:right", "occurrence:right"),
    ] {
        store.create(
            operation,
            draft(
                branch,
                Some("branch:root"),
                occurrence_id,
                Some("occurrence:root"),
                BranchStrategy::ExactRestore,
            ),
        )?;
    }
    store.attach_artifact(
        "operation:left-shared",
        "experiment:durable",
        "branch:left",
        0,
        artifact("artifact:shared", BranchArtifactRole::Checkpoint),
    )?;
    store.attach_artifact(
        "operation:left-unique",
        "experiment:durable",
        "branch:left",
        1,
        artifact("artifact:left", BranchArtifactRole::ReplayPrefix),
    )?;
    store.attach_artifact(
        "operation:right-shared",
        "experiment:durable",
        "branch:right",
        0,
        artifact("artifact:shared", BranchArtifactRole::Checkpoint),
    )?;
    store.transition(
        "operation:left-archive",
        "experiment:durable",
        "branch:left",
        2,
        DurableBranchStatus::Archived,
    )?;
    let request = BranchPruneRequest {
        experiment_id: "experiment:durable".to_owned(),
        branch_ids: vec!["branch:left".to_owned()],
        policy: BranchRetentionPolicy::default(),
    };
    let preview = store.prune_preview(&request)?;
    assert_eq!(
        preview.collectable_artifacts,
        vec![artifact("artifact:left", BranchArtifactRole::ReplayPrefix)]
    );
    assert_eq!(
        preview.retained_artifacts,
        vec![artifact("artifact:shared", BranchArtifactRole::Checkpoint)]
    );
    let plan = store.prune("operation:prune-left", &request)?;
    assert_eq!(plan, preview);
    let left = store
        .get("experiment:durable", "branch:left")?
        .expect("left");
    assert_eq!(
        left.artifacts,
        vec![artifact("artifact:shared", BranchArtifactRole::Checkpoint)]
    );
    let right = store
        .get("experiment:durable", "branch:right")?
        .expect("right");
    assert_eq!(
        right.artifacts,
        vec![artifact("artifact:shared", BranchArtifactRole::Checkpoint)]
    );
    Ok(())
}

#[test]
fn events_and_schema_migration_are_durable_and_future_versions_fail_closed()
-> Result<(), BranchStoreError> {
    let path = temp_path("migration");
    let _ = std::fs::remove_file(&path);
    let store = SqliteBranchStore::open(&path)?;
    store.create(
        "operation:root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.transition(
        "operation:archive",
        "experiment:durable",
        "branch:root",
        0,
        DurableBranchStatus::Archived,
    )?;
    let events = store.events("experiment:durable", 0, 1)?;
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.next_after_sequence, events.events[0].sequence);
    assert_eq!(events.newest_sequence, Some(events.next_after_sequence + 1));
    drop(store);
    let reopened = SqliteBranchStore::open(&path)?;
    let resumed = reopened.events("experiment:durable", events.next_after_sequence, 8)?;
    assert_eq!(resumed.events.len(), 1);
    drop(reopened);
    rusqlite::Connection::open(&path)
        .map_err(|error| BranchStoreError::Persistence(error.to_string()))?
        .execute_batch("PRAGMA user_version = 99;")
        .map_err(|error| BranchStoreError::Persistence(error.to_string()))?;
    let error = match SqliteBranchStore::open(&path) {
        Ok(_) => return Err(BranchStoreError::Corrupt),
        Err(error) => error,
    };
    assert_eq!(error, BranchStoreError::UnsupportedSchema);
    std::fs::remove_file(path).map_err(|error| BranchStoreError::Persistence(error.to_string()))?;
    Ok(())
}

#[test]
fn versioned_fixture_matches_the_owner_contract() -> Result<(), BranchStoreError> {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/durable-branch-v1/valid.json"
    ))
    .map_err(|_| BranchStoreError::Corrupt)?;
    assert_eq!(
        fixture["schema_version"],
        sts2_harness::DURABLE_BRANCH_SCHEMA_VERSION
    );
    assert_eq!(fixture["status_values"].as_array().map(Vec::len), Some(10));
    Ok(())
}
