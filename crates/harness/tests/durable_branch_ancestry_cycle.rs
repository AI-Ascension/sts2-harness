// SPDX-License-Identifier: MIT

//! Ancestry-cycle attempts against the immutable checkpoint branch tree (issue #116 AC2).
//!
//! The acceptance criterion requires an attempted cycle to be *rejected*, not merely unreachable.
//! The owner API admits three genuinely different attempts, and each is exercised here with the
//! exact rejection it produces:
//!
//! 1. a branch naming itself as its own parent (model: `InvalidLabel`; store: `InvalidInput`);
//! 2. re-parenting an existing branch id under one of its own descendants, which must reuse an
//!    already minted immutable identity (`Duplicate`) or change an existing operation payload
//!    (`IdempotencyConflict`); and
//! 3. a cycle written out of band into the owner's own database file, which the ancestry read
//!    path must refuse with `Corrupt` instead of looping.
//!
//! The boundary that makes a cycle unconstructible through the public API is named in each test:
//! a fork always mints a *new* branch identity under an *existing* parent, and the parent edge is
//! immutable, so no operation can turn a descendant into an ancestor of its own ancestor. Attempt 3
//! proves the read path detects a cycle anyway, so the guarantee does not rest on that construction
//! argument alone.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use sts2_harness::{
    BranchAssurance, BranchFork, BranchRecord, BranchStatus, BranchStoreError, BranchStrategy,
    BranchTree, BranchTreeError, DurableBranchDraft, DurableBranchStatus, ExactStateDigest,
    OccurrenceId, SqliteBranchStore,
};

const EXPERIMENT: &str = "experiment:cycle";
const ROOT: &str = "branch:cycle-root";
const CHILD: &str = "branch:cycle-child";
const ROOT_OCCURRENCE: &str = "occurrence:cycle-root";
const CHILD_OCCURRENCE: &str = "occurrence:cycle-child";
const MAX_PAGE: u64 = 128;

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
) -> DurableBranchDraft {
    DurableBranchDraft {
        experiment_id: EXPERIMENT.to_owned(),
        root_branch_id: ROOT.to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(occurrence_id),
            parent_occurrence_id: parent_occurrence_id.map(occurrence),
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
        notes: Some("synthetic ancestry cycle attempt".to_owned()),
        artifacts: Vec::new(),
    }
}

fn root_draft() -> DurableBranchDraft {
    draft(ROOT, None, ROOT_OCCURRENCE, None)
}

fn child_draft() -> DurableBranchDraft {
    draft(CHILD, Some(ROOT), CHILD_OCCURRENCE, Some(ROOT_OCCURRENCE))
}

fn record(branch_id: &str, parent: Option<&str>, scope: &str) -> BranchRecord {
    BranchRecord {
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent.map(str::to_owned),
        fork_occurrence: occurrence(&format!("occurrence:{branch_id}")),
        run_id: format!("run:{branch_id}"),
        write_scope: scope.to_owned(),
        strategy: BranchStrategy::ExactRestore,
        status: BranchStatus::Pending,
    }
}

fn scratch_database(label: &str) -> (PathBuf, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sts2-branch-cycle-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("scratch directory is creatable");
    let database = directory.join("branches.sqlite3");
    (directory, database)
}

fn remove_scratch(directory: &Path) {
    fs::remove_dir_all(directory).expect("scratch directory is removable");
}

#[test]
fn the_owner_model_refuses_a_self_parented_branch() {
    let mut tree = BranchTree::new();
    tree.create("operation:root", record(ROOT, None, "scope:root"))
        .expect("the root is admitted");
    // Attempt 1a: an existing identity naming itself as its own parent.
    assert_eq!(
        tree.create(
            "operation:self-existing",
            record(ROOT, Some(ROOT), "scope:self")
        )
        .err(),
        Some(BranchTreeError::InvalidLabel)
    );
    // Attempt 1b: a fresh identity naming itself as its own parent.
    assert_eq!(
        tree.create(
            "operation:self-new",
            record("branch:self", Some("branch:self"), "scope:self-new")
        )
        .err(),
        Some(BranchTreeError::InvalidLabel)
    );
    assert_eq!(tree.len(), 1, "a refused cycle attempt allocates nothing");
    assert_eq!(
        tree.get(ROOT)
            .and_then(|branch| branch.parent_branch_id.clone()),
        None
    );
    // The refused operation keys hold no claim: a corrected retry under the same key succeeds and
    // owns exactly one writable destination.
    let corrected = tree
        .create(
            "operation:self-new",
            record("branch:self", Some(ROOT), "scope:self-new"),
        )
        .expect("the corrected retry is admitted");
    assert_eq!(corrected.parent_branch_id.as_deref(), Some(ROOT));
    assert_eq!(corrected.write_scope, "scope:self-new");
    assert_eq!(tree.len(), 2);
}

#[test]
fn the_durable_store_refuses_a_self_parented_branch() -> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create("operation:root", root_draft())?;
    // Attempt 1: the child identity is also its own parent. Validation runs before the write
    // transaction opens, so no destination, occurrence, or operation row is ever claimed.
    let self_parented = draft(CHILD, Some(CHILD), CHILD_OCCURRENCE, Some(ROOT_OCCURRENCE));
    assert_eq!(
        store.create("operation:self-child", self_parented),
        Err(BranchStoreError::InvalidInput)
    );
    let branches = store.list(EXPERIMENT, None, MAX_PAGE)?;
    assert_eq!(branches.branches.len(), 1);
    assert!(
        store.get(EXPERIMENT, CHILD)?.is_none(),
        "a refused self-parent claims no child destination"
    );
    let root = store
        .get(EXPERIMENT, ROOT)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(root.parent_branch_id, None);
    assert_eq!(root.metadata_revision, 0);
    assert_eq!(
        store.events(EXPERIMENT, 0, MAX_PAGE)?.newest_sequence,
        Some(1)
    );
    // The same operation key is free for the corrected fork.
    let corrected = store.create("operation:self-child", child_draft())?;
    assert_eq!(corrected.parent_branch_id.as_deref(), Some(ROOT));
    assert_eq!(corrected.status, DurableBranchStatus::Pending);
    Ok(())
}

#[test]
fn an_existing_branch_cannot_be_re_parented_under_its_own_descendant()
-> Result<(), BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create("operation:root", root_draft())?;
    store.create("operation:child", child_draft())?;
    // Attempt 2a: make the root a child of its own descendant. A fork always mints a new branch
    // identity, so this can only reuse an already minted, immutable identity.
    let descendant_parent = draft(ROOT, Some(CHILD), ROOT_OCCURRENCE, Some(CHILD_OCCURRENCE));
    assert_eq!(
        store.create("operation:reparent-root", descendant_parent),
        Err(BranchStoreError::Duplicate)
    );
    // Attempt 2b: reuse the root's own operation key with a changed, cyclic payload.
    let changed_payload = draft(ROOT, Some(CHILD), ROOT_OCCURRENCE, Some(CHILD_OCCURRENCE));
    assert_eq!(
        store.create("operation:root", changed_payload),
        Err(BranchStoreError::IdempotencyConflict)
    );
    // The immutable parent edges are unchanged: no cycle exists.
    let root = store
        .get(EXPERIMENT, ROOT)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(root.parent_branch_id, None);
    let child = store
        .get(EXPERIMENT, CHILD)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(child.parent_branch_id.as_deref(), Some(ROOT));
    let root_ancestry: Vec<String> = store
        .ancestry(EXPERIMENT, ROOT)?
        .into_iter()
        .map(|branch| branch.branch_id)
        .collect();
    assert_eq!(root_ancestry, [ROOT.to_owned()]);
    let child_ancestry: Vec<String> = store
        .ancestry(EXPERIMENT, CHILD)?
        .into_iter()
        .map(|branch| branch.branch_id)
        .collect();
    assert_eq!(child_ancestry, [ROOT.to_owned(), CHILD.to_owned()]);
    assert_eq!(store.list(EXPERIMENT, None, MAX_PAGE)?.branches.len(), 2);
    Ok(())
}

#[test]
fn a_cycled_ancestry_is_rejected_with_corrupt_instead_of_looping() -> Result<(), BranchStoreError> {
    let (directory, database) = scratch_database("corrupt");
    {
        let store = SqliteBranchStore::open(&database)?;
        store.create("operation:root", root_draft())?;
        store.create("operation:child", child_draft())?;
    }
    // Close the cycle out of band in the owner's own file: the root becomes the child's child.
    let raw = Connection::open(&database).expect("raw connection opens");
    raw.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("the raw connection relaxes foreign keys");
    raw.execute(
        "UPDATE durable_branches SET parent_branch_id = ?2
         WHERE experiment_id = ?1 AND branch_id = ?3",
        params![EXPERIMENT, CHILD, ROOT],
    )
    .expect("the cycle is written out of band");
    drop(raw);
    {
        let store = SqliteBranchStore::open(&database)?;
        // The read path refuses the cycle with a specific error instead of looping or truncating.
        assert_eq!(
            store.ancestry(EXPERIMENT, ROOT).unwrap_err(),
            BranchStoreError::Corrupt
        );
        assert_eq!(
            store.ancestry(EXPERIMENT, CHILD).unwrap_err(),
            BranchStoreError::Corrupt
        );
        // Detection propagates to anything reached through the cyclic edge.
        store.create(
            "operation:grandchild",
            draft(
                "branch:cycle-grandchild",
                Some(CHILD),
                "occurrence:cycle-grandchild",
                Some(CHILD_OCCURRENCE),
            ),
        )?;
        assert_eq!(
            store
                .ancestry(EXPERIMENT, "branch:cycle-grandchild")
                .unwrap_err(),
            BranchStoreError::Corrupt
        );
        // A direct read is still available and the public API still refuses a cycle attempt.
        assert!(
            store.get(EXPERIMENT, ROOT)?.is_some(),
            "the cyclic record is readable, its ancestry is not"
        );
        let repeated = draft(ROOT, Some(CHILD), ROOT_OCCURRENCE, Some(CHILD_OCCURRENCE));
        assert_eq!(
            store.create("operation:cycle-again", repeated),
            Err(BranchStoreError::Duplicate)
        );
    }
    remove_scratch(&directory);
    Ok(())
}
