// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use sts2_harness::{
    BranchRecord, BranchStatus, BranchStrategy, BranchTree, BranchTreeError, OccurrenceId,
};

fn occurrence(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("valid occurrence")
}

fn branch(id: &str, parent: Option<&str>, scope: &str) -> BranchRecord {
    BranchRecord {
        branch_id: id.to_owned(),
        parent_branch_id: parent.map(str::to_owned),
        fork_occurrence: occurrence(&format!("occurrence:{id}")),
        run_id: format!("run:{id}"),
        write_scope: scope.to_owned(),
        strategy: BranchStrategy::ExactRestore,
        status: BranchStatus::Pending,
    }
}

#[test]
fn siblings_remain_distinct_and_a_retry_cannot_rebind_them() {
    let mut tree = BranchTree::new();
    let root = branch("branch:root", None, "scope:root");
    tree.create("operation:root", root).expect("root");
    let left = branch("branch:left", Some("branch:root"), "scope:left");
    tree.create("operation:left", left.clone()).expect("left");
    tree.transition("branch:left", BranchStatus::Preparing)
        .expect("prepare");
    assert_eq!(
        tree.create("operation:left", left)
            .expect("idempotent retry")
            .status,
        BranchStatus::Preparing
    );
    let right = branch("branch:right", Some("branch:root"), "scope:right");
    tree.create("operation:right", right).expect("right");
    assert_eq!(tree.len(), 3);
    assert_ne!(tree.get("branch:left"), tree.get("branch:right"));
    assert_eq!(
        tree.create(
            "operation:left",
            branch("branch:other", Some("branch:root"), "scope:other")
        )
        .expect_err("changed retry rejects"),
        BranchTreeError::IdempotencyConflict
    );
}

#[test]
fn unknown_parent_and_shared_writable_scope_are_rejected() {
    let mut tree = BranchTree::new();
    assert_eq!(
        tree.create(
            "operation:child",
            branch("branch:child", Some("branch:missing"), "scope:child")
        )
        .expect_err("parent missing"),
        BranchTreeError::UnknownParent
    );
    tree.create("operation:root", branch("branch:root", None, "scope:root"))
        .expect("root");
    assert_eq!(
        tree.create(
            "operation:child",
            branch("branch:child", Some("branch:root"), "scope:root")
        )
        .expect_err("scope repeats"),
        BranchTreeError::Duplicate
    );
}

#[test]
fn lifecycle_never_promotes_unprepared_or_archived_work() {
    let mut tree = BranchTree::new();
    tree.create("operation:root", branch("branch:root", None, "scope:root"))
        .expect("root");
    assert_eq!(
        tree.transition("branch:root", BranchStatus::Ready)
            .expect_err("pending cannot be ready"),
        BranchTreeError::InvalidTransition
    );
    tree.transition("branch:root", BranchStatus::Preparing)
        .expect("prepare");
    tree.transition("branch:root", BranchStatus::Ready)
        .expect("ready");
    tree.transition("branch:root", BranchStatus::Archived)
        .expect("archive");
    assert_eq!(
        tree.transition("branch:root", BranchStatus::Ready)
            .expect_err("archive needs explicit reactivation"),
        BranchTreeError::InvalidTransition
    );
}

#[test]
fn equal_game_state_content_does_not_define_branch_identity() {
    let mut tree = BranchTree::new();
    tree.create("operation:root", branch("branch:root", None, "scope:root"))
        .expect("root");
    tree.create(
        "operation:left",
        branch("branch:left", Some("branch:root"), "scope:left"),
    )
    .expect("left");
    tree.create(
        "operation:right",
        branch("branch:right", Some("branch:root"), "scope:right"),
    )
    .expect("right");
    let branch_ids: Vec<&str> = tree
        .branches()
        .into_iter()
        .map(|record| record.branch_id.as_str())
        .collect();
    assert_eq!(branch_ids, ["branch:left", "branch:right", "branch:root"]);
}
