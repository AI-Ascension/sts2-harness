// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use sts2_harness::{
    BranchAssurance, BranchFork, BranchStoreError, BranchStrategy, DurableBranchDraft,
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
    strategy: BranchStrategy,
) -> DurableBranchDraft {
    DurableBranchDraft {
        experiment_id: "experiment:durable".to_owned(),
        root_branch_id: "branch:root".to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(&format!("occurrence:{branch_id}")),
            parent_occurrence_id: Some(occurrence("occurrence:branch:root")),
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

fn root_draft() -> DurableBranchDraft {
    let mut root = draft("branch:root", None, BranchStrategy::ExactRestore);
    root.fork.parent_occurrence_id = None;
    root
}

/// A child whose branch id may exceed the bounded occurrence-id length, so the occurrence id is
/// supplied explicitly and separately.
fn long_child_draft(branch_id: &str) -> DurableBranchDraft {
    let mut child = draft(
        "branch:template",
        Some("branch:root"),
        BranchStrategy::ExactRestore,
    );
    child.branch_id = branch_id.to_owned();
    child.fork.occurrence_id = occurrence("occurrence:long");
    child.source_handle = Some("source:long".to_owned());
    child.run_id = "run:long".to_owned();
    child.episode_id = Some("episode:long".to_owned());
    child.trajectory_id = Some("trajectory:long".to_owned());
    child.context_id = Some("context:long".to_owned());
    child.name = "long".to_owned();
    child
}

#[test]
fn startup_reconciliation_resolves_half_created_branches_deterministically()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create("operation:root", root_draft())?;
    // A half-created exact restore that started but never reached ready.
    store.create(
        "operation:restoring",
        draft(
            "branch:restoring",
            Some("branch:root"),
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.transition(
        "operation:to-restoring",
        "experiment:durable",
        "branch:restoring",
        0,
        DurableBranchStatus::Restoring,
    )?;
    // A half-created prefix replay whose effect is unknown.
    store.create(
        "operation:unknown",
        draft(
            "branch:unknown",
            Some("branch:root"),
            BranchStrategy::PrefixReplay,
        ),
    )?;
    store.transition(
        "operation:to-replaying",
        "experiment:durable",
        "branch:unknown",
        0,
        DurableBranchStatus::Replaying,
    )?;
    store.transition(
        "operation:to-unknown",
        "experiment:durable",
        "branch:unknown",
        1,
        DurableBranchStatus::Unknown,
    )?;
    // A fully ready branch must not be touched by reconciliation.
    store.create(
        "operation:ready-create",
        draft(
            "branch:ready",
            Some("branch:root"),
            BranchStrategy::ExactRestore,
        ),
    )?;
    store.transition(
        "operation:ready-restoring",
        "experiment:durable",
        "branch:ready",
        0,
        DurableBranchStatus::Restoring,
    )?;
    store.set_assurance(
        "operation:ready-assurance",
        "experiment:durable",
        "branch:ready",
        1,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    store.transition(
        "operation:ready",
        "experiment:durable",
        "branch:ready",
        2,
        DurableBranchStatus::Ready,
    )?;

    // The unresolved set is exactly the half-created branches.
    let candidates = store.reconciliation_candidates("experiment:durable")?;
    let candidate_ids = candidates
        .iter()
        .map(|branch| branch.branch_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        candidate_ids,
        ["branch:restoring", "branch:root", "branch:unknown"]
    );

    let mut resolved = store.reconcile_startup("reconcile:startup", "experiment:durable")?;
    resolved.sort_by(|left, right| left.branch_id.cmp(&right.branch_id));
    let summary = resolved
        .iter()
        .map(|branch| (branch.branch_id.as_str(), branch.status))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            ("branch:restoring", DurableBranchStatus::Failed),
            ("branch:root", DurableBranchStatus::Archived),
            ("branch:unknown", DurableBranchStatus::Failed),
        ]
    );
    assert_eq!(
        store
            .get("experiment:durable", "branch:ready")?
            .map(|branch| branch.status),
        Some(DurableBranchStatus::Ready)
    );

    // Reconciliation is idempotent: a retry is a no-op and no candidate remains.
    assert!(
        store
            .reconcile_startup("reconcile:startup", "experiment:durable")?
            .is_empty()
    );
    assert!(
        store
            .reconciliation_candidates("experiment:durable")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn reconciliation_handles_a_maximum_length_branch_id() -> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create("operation:root", root_draft())?;
    let long_id = format!("branch:{}", "x".repeat(249));
    assert_eq!(long_id.len(), 256, "branch id is at the label bound");
    store.create("operation:long-create", long_child_draft(&long_id))?;
    store.transition(
        "operation:long-restoring",
        "experiment:durable",
        &long_id,
        0,
        DurableBranchStatus::Restoring,
    )?;

    let resolved = store.reconcile_startup("reconcile:startup", "experiment:durable")?;
    assert_eq!(resolved.len(), 2);
    assert!(resolved.iter().any(|branch| {
        branch.branch_id == long_id && branch.status == DurableBranchStatus::Failed
    }));
    assert!(resolved.iter().any(|branch| {
        branch.branch_id == "branch:root" && branch.status == DurableBranchStatus::Archived
    }));
    Ok(())
}
