// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::PathBuf;

use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchFork, BranchPruneRequest,
    BranchRetentionPolicy, BranchStoreError, BranchStrategy, DurableBranchDraft,
    DurableBranchStatus, ExactStateDigest, MAX_BRANCHES, OccurrenceId, SqliteBranchStore,
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
    draft_for(
        "experiment:durable",
        "branch:root",
        branch_id,
        parent_branch_id,
        occurrence_id,
        parent_occurrence_id,
        strategy,
    )
}

fn draft_for(
    experiment_id: &str,
    root_branch_id: &str,
    branch_id: &str,
    parent_branch_id: Option<&str>,
    occurrence_id: &str,
    parent_occurrence_id: Option<&str>,
    strategy: BranchStrategy,
) -> DurableBranchDraft {
    DurableBranchDraft {
        experiment_id: experiment_id.to_owned(),
        root_branch_id: root_branch_id.to_owned(),
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
fn branch_capacity_is_transactional() -> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:capacity-root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    for index in 1..MAX_BRANCHES {
        let branch_id = format!("branch:capacity-{index}");
        let occurrence_id = format!("occurrence:capacity-{index}");
        store.create(
            &format!("operation:capacity-{index}"),
            draft(
                &branch_id,
                Some("branch:root"),
                &occurrence_id,
                Some("occurrence:root"),
                BranchStrategy::ExactRestore,
            ),
        )?;
    }
    assert_eq!(
        store.create(
            "operation:capacity-overflow",
            draft(
                "branch:capacity-overflow",
                Some("branch:root"),
                "occurrence:capacity-overflow",
                Some("occurrence:root"),
                BranchStrategy::ExactRestore,
            ),
        ),
        Err(BranchStoreError::Capacity)
    );
    assert_eq!(
        store
            .list("experiment:durable", None, MAX_BRANCHES as u64)?
            .branches
            .len(),
        MAX_BRANCHES
    );
    Ok(())
}

#[test]
fn concurrent_creates_share_the_same_capacity_and_scope() -> Result<(), BranchStoreError> {
    let path = temp_path("concurrent");
    let _ = std::fs::remove_file(&path);
    let store = SqliteBranchStore::open(&path)?;
    store.create(
        "operation:concurrent-root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    drop(store);
    let left = std::sync::Arc::new(SqliteBranchStore::open(&path)?);
    let right = std::sync::Arc::new(SqliteBranchStore::open(&path)?);
    let left_thread = {
        let store = std::sync::Arc::clone(&left);
        std::thread::spawn(move || {
            store.create(
                "operation:concurrent-left",
                draft(
                    "branch:concurrent-left",
                    Some("branch:root"),
                    "occurrence:concurrent-left",
                    Some("occurrence:root"),
                    BranchStrategy::ExactRestore,
                ),
            )
        })
    };
    let right_thread = {
        let store = std::sync::Arc::clone(&right);
        std::thread::spawn(move || {
            store.create(
                "operation:concurrent-right",
                draft(
                    "branch:concurrent-right",
                    Some("branch:root"),
                    "occurrence:concurrent-right",
                    Some("occurrence:root"),
                    BranchStrategy::PrefixReplay,
                ),
            )
        })
    };
    left_thread
        .join()
        .map_err(|_| BranchStoreError::Corrupt)??;
    right_thread
        .join()
        .map_err(|_| BranchStoreError::Corrupt)??;
    assert_eq!(
        left.list("experiment:durable", None, MAX_BRANCHES as u64)?
            .branches
            .len(),
        3
    );
    drop(left);
    drop(right);
    std::fs::remove_file(path).map_err(|error| BranchStoreError::Persistence(error.to_string()))?;
    Ok(())
}

#[test]
fn concurrent_identical_creates_share_one_idempotent_child() -> Result<(), BranchStoreError> {
    let path = temp_path("concurrent-identical");
    let _ = std::fs::remove_file(&path);
    let store = SqliteBranchStore::open(&path)?;
    store.create(
        "operation:concurrent-identical-root",
        draft(
            "branch:root",
            None,
            "occurrence:root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    drop(store);

    let left = std::sync::Arc::new(SqliteBranchStore::open(&path)?);
    let right = std::sync::Arc::new(SqliteBranchStore::open(&path)?);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let payload = draft(
        "branch:concurrent-identical",
        Some("branch:root"),
        "occurrence:concurrent-identical",
        Some("occurrence:root"),
        BranchStrategy::PrefixReplay,
    );
    let left_thread = {
        let store = std::sync::Arc::clone(&left);
        let barrier = std::sync::Arc::clone(&barrier);
        let payload = payload.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.create("operation:concurrent-identical", payload)
        })
    };
    let right_thread = {
        let store = std::sync::Arc::clone(&right);
        let barrier = std::sync::Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            store.create("operation:concurrent-identical", payload)
        })
    };
    barrier.wait();
    let left_branch = left_thread
        .join()
        .map_err(|_| BranchStoreError::Corrupt)??;
    let right_branch = right_thread
        .join()
        .map_err(|_| BranchStoreError::Corrupt)??;
    assert_eq!(left_branch, right_branch);
    assert_eq!(left_branch.branch_id, "branch:concurrent-identical");
    assert_eq!(left_branch.parent_branch_id.as_deref(), Some("branch:root"));

    drop(left);
    drop(right);
    let reopened = SqliteBranchStore::open(&path)?;
    let page = reopened.list("experiment:durable", None, MAX_BRANCHES as u64)?;
    assert_eq!(
        page.branches
            .iter()
            .filter(|branch| branch.branch_id == "branch:concurrent-identical")
            .count(),
        1
    );
    let events = reopened.events("experiment:durable", 0, MAX_BRANCHES as u64)?;
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| {
                event.branch_id == "branch:concurrent-identical"
                    && event.operation_id == "operation:concurrent-identical"
            })
            .count(),
        1
    );
    std::fs::remove_file(path).map_err(|error| BranchStoreError::Persistence(error.to_string()))?;
    Ok(())
}

#[test]
fn artifact_identity_is_shared_across_roles_and_experiments() -> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:durable-root",
        draft_for(
            "experiment:durable",
            "branch:durable-root",
            "branch:durable-root",
            None,
            "occurrence:durable-root",
            None,
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.create(
        "operation:durable-child",
        draft_for(
            "experiment:durable",
            "branch:durable-root",
            "branch:durable-child",
            Some("branch:durable-root"),
            "occurrence:durable-child",
            Some("occurrence:durable-root"),
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.create(
        "operation:other-root",
        draft_for(
            "experiment:other",
            "branch:other-root",
            "branch:other-root",
            None,
            "occurrence:other-root",
            None,
            BranchStrategy::PrefixReplay,
        ),
    )?;
    store.attach_artifact(
        "operation:durable-artifact",
        "experiment:durable",
        "branch:durable-child",
        0,
        artifact("artifact:shared", BranchArtifactRole::Checkpoint),
    )?;
    store.attach_artifact(
        "operation:other-artifact",
        "experiment:other",
        "branch:other-root",
        0,
        artifact("artifact:shared", BranchArtifactRole::ReplayPrefix),
    )?;
    store.transition(
        "operation:durable-archive",
        "experiment:durable",
        "branch:durable-child",
        1,
        DurableBranchStatus::Archived,
    )?;
    store.transition(
        "operation:other-archive",
        "experiment:other",
        "branch:other-root",
        1,
        DurableBranchStatus::Archived,
    )?;

    let request = BranchPruneRequest {
        experiment_id: "experiment:durable".to_owned(),
        branch_ids: vec!["branch:durable-child".to_owned()],
        policy: BranchRetentionPolicy::default(),
    };
    let preview = store.prune_preview(&request)?;
    assert!(preview.collectable_artifacts.is_empty());
    assert_eq!(
        preview.retained_artifacts,
        vec![
            artifact("artifact:shared", BranchArtifactRole::Checkpoint),
            artifact("artifact:shared", BranchArtifactRole::ReplayPrefix),
        ]
    );
    store.prune("operation:durable-prune", &request)?;

    let other_request = BranchPruneRequest {
        experiment_id: "experiment:other".to_owned(),
        branch_ids: vec!["branch:other-root".to_owned()],
        policy: BranchRetentionPolicy::default(),
    };
    let other_plan = store.prune("operation:other-prune", &other_request)?;
    assert_eq!(
        other_plan.collectable_artifacts,
        vec![artifact(
            "artifact:shared",
            BranchArtifactRole::ReplayPrefix
        )]
    );
    Ok(())
}
