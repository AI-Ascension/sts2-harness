// SPDX-License-Identifier: MIT

//! Rollback, refusal and destination-ownership recovery for durable branch persistence.
//!
//! These are the persistence steps that never commit at all, so they must leave nothing behind:
//!
//! - a deterministic write failure injected *inside* the fork-intent transaction after an
//!   earlier write of the same transaction, proving the rollback is total;
//! - a refused demonstration of the ready-publication step, shown to leave the metadata
//!   revision, the assurance and the event log untouched; and
//! - the artifact store's staging destinations, which must never stay unowned.
//!
//! Removal of stale staging destinations is already covered by
//! `crates/harness/tests/exact_checkpoint_retention.rs::stale_staging_files_are_recovered`; the
//! test here adds the ownership and idempotence assertions for that same boundary instead of
//! repeating the removal assertion alone.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use sts2_harness::{
    BranchAssurance, BranchStoreError, DurableBranchStatus, ExactArtifactStore, SqliteBranchStore,
};

#[path = "support/durable_branch_crash.rs"]
mod support;

use support::*;

#[test]
fn interrupted_fork_intent_rolls_back_without_claiming_a_destination()
-> Result<(), BranchStoreError> {
    let directory = scratch_directory("interrupted-intent");
    let database = directory.join("branches.sqlite3");
    {
        let store = SqliteBranchStore::open(&database)?;
        store.create("operation:crash-root", root_draft())?;
        // The child claims the run destination the root already owns, so the branch insert fails
        // after the fork occurrence was already written inside the same transaction.
        let mut conflicting = child_draft();
        conflicting.run_id = ROOT_RUN.to_owned();
        assert_eq!(
            store.create("operation:rollback-child", conflicting),
            Err(BranchStoreError::Duplicate)
        );
        assert!(store.get(EXPERIMENT, CHILD)?.is_none());
        assert_eq!(store.list(EXPERIMENT, None, MAX_PAGE)?.branches.len(), 1);
        assert_eq!(
            table_rows(&database, "branch_occurrences"),
            1,
            "the pre-write occurrence insert rolled back with the intent"
        );
        assert_eq!(
            table_rows(&database, "branch_operations"),
            1,
            "no operation row survives a rolled-back intent"
        );
        assert_eq!(orphan_rows(&database), OrphanRows::default());
        // The same operation key succeeds once its destination is not already claimed.
        let created = store.create("operation:rollback-child", child_draft())?;
        assert_eq!(created.status, DurableBranchStatus::Pending);
        assert_eq!(created.run_id, CHILD_RUN);
        assert_eq!(created.metadata_revision, 0);
        assert_eq!(orphan_rows(&database), OrphanRows::default());
    }
    assert!(leftover_sidecars(&database).is_empty());
    assert!(stray_entries(&directory).is_empty());
    remove_scratch(&directory);
    Ok(())
}

#[test]
fn refused_publications_never_advance_the_revision_or_the_event_log() -> Result<(), BranchStoreError>
{
    let store = SqliteBranchStore::open_in_memory()?;
    store.create("operation:crash-root", root_draft())?;
    store.create("operation:crash-child", child_draft())?;
    store.attach_artifact(
        "operation:crash-artifact",
        EXPERIMENT,
        CHILD,
        0,
        checkpoint_artifact(),
    )?;
    store.transition(
        "operation:crash-restoring",
        EXPERIMENT,
        CHILD,
        1,
        DurableBranchStatus::Restoring,
    )?;
    assert_eq!(
        store.transition(
            "operation:early-ready",
            EXPERIMENT,
            CHILD,
            2,
            DurableBranchStatus::Ready
        ),
        Err(BranchStoreError::InsufficientAssurance)
    );
    store.set_assurance(
        "operation:crash-assurance",
        EXPERIMENT,
        CHILD,
        2,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    let before = store
        .get(EXPERIMENT, CHILD)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(before.status, DurableBranchStatus::Restoring);
    assert_eq!(before.metadata_revision, 3);
    let sequence = store
        .events(EXPERIMENT, 0, MAX_PAGE)?
        .newest_sequence
        .expect("the child already recorded events");
    assert_eq!(
        store.transition(
            "operation:stale-ready",
            EXPERIMENT,
            CHILD,
            2,
            DurableBranchStatus::Ready
        ),
        Err(BranchStoreError::StaleRevision)
    );
    assert_eq!(
        store.transition(
            "operation:jump",
            EXPERIMENT,
            CHILD,
            3,
            DurableBranchStatus::Completed
        ),
        Err(BranchStoreError::InvalidTransition)
    );
    let after = store
        .get(EXPERIMENT, CHILD)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(after.status, before.status);
    assert_eq!(after.metadata_revision, before.metadata_revision);
    assert_eq!(after.assurance, before.assurance);
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(
        store.events(EXPERIMENT, 0, MAX_PAGE)?.newest_sequence,
        Some(sequence),
        "a refused publication appends no event"
    );
    // The publication then commits exactly once at the owning revision.
    let published = store.transition(
        "operation:ready",
        EXPERIMENT,
        CHILD,
        3,
        DurableBranchStatus::Ready,
    )?;
    assert_eq!(published.status, DurableBranchStatus::Ready);
    assert_eq!(published.metadata_revision, 4);
    assert_eq!(published.assurance, BranchAssurance::ExactRestoreReceipt);
    assert_eq!(
        store.events(EXPERIMENT, 0, MAX_PAGE)?.newest_sequence,
        Some(sequence + 1)
    );
    Ok(())
}

/// Walks a store root for staging destinations left by an interrupted publication.
fn staging_leftovers(directory: &Path) -> Vec<PathBuf> {
    let mut leftovers = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".tmp-"))
            {
                leftovers.push(path);
            }
        }
    }
    leftovers
}

#[test]
fn unowned_staging_destinations_are_recovered_idempotently() {
    let directory = scratch_directory("staging");
    let store = ExactArtifactStore::new(directory.join("artifacts"));
    let owned = store.stage_blob(b"owned").expect("blob stages");
    let hex = owned.as_str().trim_start_matches("sha256:").to_owned();
    let blob_prefix = store
        .root_directory()
        .join("exact")
        .join("blobs")
        .join(&hex[..2]);
    fs::create_dir_all(&blob_prefix).expect("blob prefix directory is creatable");
    fs::write(blob_prefix.join(".tmp-1111"), b"partial-blob").expect("staged blob writes");
    let manifest_prefix = store
        .root_directory()
        .join("exact")
        .join("manifests")
        .join("aa");
    fs::create_dir_all(&manifest_prefix).expect("manifest prefix directory is creatable");
    fs::write(manifest_prefix.join(".tmp-2222"), b"partial-manifest")
        .expect("staged manifest writes");
    assert_eq!(
        store.recover_temporaries().expect("recovery runs"),
        2,
        "exactly the unowned staging destinations are removed"
    );
    assert_eq!(
        store.recover_temporaries().expect("second recovery runs"),
        0,
        "recovery is idempotent and leaves nothing unowned"
    );
    assert_eq!(
        store.stored_blobs().expect("blobs list"),
        vec![owned.clone()],
        "the published blob is the only retained destination"
    );
    assert_eq!(store.read_blob(&owned).expect("owned blob reads"), b"owned");
    assert!(
        staging_leftovers(store.root_directory()).is_empty(),
        "no unowned staging destination remains under the store root"
    );
    remove_scratch(&directory);
}
