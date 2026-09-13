// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

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

fn draft(branch_id: &str, parent: Option<&str>, occurrence_id: &str) -> DurableBranchDraft {
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
        artifacts: Vec::new(),
    }
}

fn artifact(id: &str, role: BranchArtifactRole) -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: id.to_owned(),
        role,
    }
}

#[test]
fn artifact_prune_keeps_shared_dependencies_and_tombstones_collectable_edges()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:root",
        draft("branch:root", None, "occurrence:root"),
    )?;
    for (operation, branch, occurrence_id) in [
        ("operation:left", "branch:left", "occurrence:left"),
        ("operation:right", "branch:right", "occurrence:right"),
    ] {
        store.create(operation, draft(branch, Some("branch:root"), occurrence_id))?;
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
    assert!(
        store
            .get("experiment:durable", "branch:left")?
            .expect("left")
            .artifacts
            .is_empty()
    );
    assert_eq!(
        store
            .get("experiment:durable", "branch:right")?
            .expect("right")
            .artifacts,
        vec![artifact("artifact:shared", BranchArtifactRole::Checkpoint)]
    );
    assert_eq!(
        store
            .transition(
                "operation:left-reactivate",
                "experiment:durable",
                "branch:left",
                3,
                DurableBranchStatus::Pending,
            )
            .expect_err("tombstoned branch cannot be reactivated"),
        BranchStoreError::InvalidTransition
    );
    assert_eq!(
        store
            .attach_artifact(
                "operation:left-reuse",
                "experiment:durable",
                "branch:left",
                3,
                artifact("artifact:new", BranchArtifactRole::Checkpoint),
            )
            .expect_err("tombstoned branch cannot gain new roots"),
        BranchStoreError::InvalidTransition
    );
    store.transition(
        "operation:right-archive",
        "experiment:durable",
        "branch:right",
        1,
        DurableBranchStatus::Archived,
    )?;
    let right_plan = store.prune(
        "operation:prune-right",
        &BranchPruneRequest {
            experiment_id: "experiment:durable".to_owned(),
            branch_ids: vec!["branch:right".to_owned()],
            policy: BranchRetentionPolicy::default(),
        },
    )?;
    assert_eq!(
        right_plan.collectable_artifacts,
        vec![artifact("artifact:shared", BranchArtifactRole::Checkpoint)]
    );
    assert_eq!(
        store.prune("operation:prune-left", &request)?,
        plan,
        "retry replays the original plan after reachability changed"
    );
    Ok(())
}
