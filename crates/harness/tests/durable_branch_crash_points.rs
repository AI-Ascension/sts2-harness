// SPDX-License-Identifier: MIT

//! Crash-point fault injection for durable checkpoint branch persistence.
//!
//! Issue #116 AC3 names four persistence crash points in commit order: fork intent, artifact
//! association, strategy completion, and ready publication. Every owner step is one
//! `BEGIN IMMEDIATE` transaction committed exactly once, so a crash inside a step either rolls
//! back (leaving the pre-step state) or has already produced a complete commit. The durable
//! partial states a crash can leave are therefore the step boundaries, and this suite proves each
//! of them with a real crashing process that dies by `SIGABRT` without closing the store, so the
//! asserting process reopens an unclean WAL rather than a cleanly closed file.
//!
//! No writable destination is left unowned after any of these crashes: the crashed writer's
//! WAL/SHM destinations are reclaimed, no retained row forms an orphan edge, and every run
//! destination keeps exactly one owner. The remaining failure modes that never commit at all -- a
//! rolled-back fork intent, a refused ready publication and the artifact staging destinations --
//! live in `durable_branch_crash_recovery.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use sts2_harness::{BranchAssurance, BranchStoreError, DurableBranchStatus, SqliteBranchStore};

#[path = "support/durable_branch_crash.rs"]
mod support;

use support::*;

/// Writes the state a crash at `point` leaves, then dies without closing the store.
#[test]
#[ignore = "launched explicitly by crash_at_every_persistence_boundary_recovers_one_owned_child"]
fn crash_worker() {
    let Some(database) = std::env::var_os("STS2_BRANCH_CRASH_DB") else {
        return;
    };
    let point = std::env::var("STS2_BRANCH_CRASH_POINT")
        .expect("crash point is set")
        .parse::<u8>()
        .expect("crash point parses");
    let store = SqliteBranchStore::open(PathBuf::from(database)).expect("worker opens the store");
    store
        .create("operation:crash-root", root_draft())
        .expect("root commits");
    // Crash point 1: fork intent committed, nothing else.
    store
        .create("operation:crash-child", child_draft())
        .expect("fork intent commits");
    if point <= 1 {
        std::process::abort();
    }
    // Crash point 2: artifact association committed.
    store
        .attach_artifact(
            "operation:crash-artifact",
            EXPERIMENT,
            CHILD,
            0,
            checkpoint_artifact(),
        )
        .expect("artifact association commits");
    if point <= 2 {
        std::process::abort();
    }
    // Crash point 3: the strategy started and recorded its completion evidence.
    store
        .transition(
            "operation:crash-restoring",
            EXPERIMENT,
            CHILD,
            1,
            DurableBranchStatus::Restoring,
        )
        .expect("strategy start commits");
    store
        .set_assurance(
            "operation:crash-assurance",
            EXPERIMENT,
            CHILD,
            2,
            BranchAssurance::ExactRestoreReceipt,
        )
        .expect("strategy completion commits");
    if point <= 3 {
        std::process::abort();
    }
    // Crash point 4: ready publication committed, then the process died.
    store
        .transition(
            "operation:crash-ready",
            EXPERIMENT,
            CHILD,
            3,
            DurableBranchStatus::Ready,
        )
        .expect("ready publication commits");
    std::process::abort();
}

fn crash_at(point: u8) -> (PathBuf, PathBuf) {
    let directory = scratch_directory(&format!("point-{point}"));
    let database = directory.join("branches.sqlite3");
    let status = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "crash_worker", "--ignored"])
        .env("STS2_BRANCH_CRASH_DB", &database)
        .env("STS2_BRANCH_CRASH_POINT", point.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("crash worker spawns");
    assert!(
        !status.success(),
        "crash worker at point {point} must not exit cleanly"
    );
    assert!(database.is_file(), "crash point {point} left a store file");
    (directory, database)
}

#[test]
fn crash_at_every_persistence_boundary_recovers_one_owned_child() -> Result<(), BranchStoreError> {
    for (point, label) in CRASH_POINTS {
        let (directory, database) = crash_at(point);
        {
            let store = SqliteBranchStore::open(&database)?;
            // The committed half-created child is exactly what the crashed writer owned.
            let (status, revision, artifacts) = committed_state(point);
            let child = store
                .get(EXPERIMENT, CHILD)?
                .ok_or(BranchStoreError::UnknownBranch)?;
            assert_eq!(
                child.status, status,
                "crash point {point} ({label}): status"
            );
            assert_eq!(
                child.metadata_revision, revision,
                "crash point {point} ({label}): metadata revision"
            );
            assert_eq!(
                child.artifacts.len(),
                artifacts,
                "crash point {point} ({label}): retained artifacts"
            );
            assert_eq!(
                child.run_id, CHILD_RUN,
                "crash point {point} ({label}): run destination owner"
            );
            if point >= 3 {
                assert_eq!(
                    child.assurance,
                    BranchAssurance::ExactRestoreReceipt,
                    "crash point {point} ({label}): completion evidence"
                );
            }
            // A retried fork intent is idempotent: it never mints a second child.
            let retried = store.create("operation:crash-child", child_draft())?;
            assert_eq!(
                retried.branch_id, CHILD,
                "crash point {point}: retry identity"
            );
            let mut conflicting = child_draft();
            conflicting.run_id = "run:crash-conflict".to_owned();
            assert_eq!(
                store.create("operation:crash-child", conflicting),
                Err(BranchStoreError::IdempotencyConflict),
                "crash point {point} ({label}): a changed retry cannot rebind the destination"
            );
            let page = store.list(EXPERIMENT, None, MAX_PAGE)?;
            assert_eq!(
                page.branches
                    .iter()
                    .filter(|branch| branch.branch_id == CHILD)
                    .count(),
                1,
                "crash point {point} ({label}): exactly one child"
            );
            assert_eq!(
                page.branches.len(),
                2,
                "crash point {point} ({label}): root plus one child"
            );
            let runs: BTreeSet<&str> = page
                .branches
                .iter()
                .map(|branch| branch.run_id.as_str())
                .collect();
            assert_eq!(
                runs.len(),
                page.branches.len(),
                "crash point {point} ({label}): every run destination has one owner"
            );
            // No retained edge is unowned.
            assert_eq!(
                orphan_rows(&database),
                OrphanRows::default(),
                "crash point {point} ({label}): orphan rows"
            );
            // Startup reconciliation fails the half-created child closed, deterministically.
            let resolved = store.reconcile_startup("reconcile:crash", EXPERIMENT)?;
            let resolved_ids: Vec<&str> = resolved
                .iter()
                .map(|branch| branch.branch_id.as_str())
                .collect();
            let expected_ids: Vec<&str> = if point == 4 {
                vec![ROOT]
            } else {
                vec![CHILD, ROOT]
            };
            assert_eq!(
                resolved_ids, expected_ids,
                "crash point {point} ({label}): reconciliation set"
            );
            let settled = store
                .get(EXPERIMENT, CHILD)?
                .ok_or(BranchStoreError::UnknownBranch)?;
            assert_eq!(
                settled.status,
                reconciled_status(point),
                "crash point {point} ({label}): reconciled status"
            );
            assert_eq!(
                settled.metadata_revision,
                revision + u64::from(point != 4),
                "crash point {point} ({label}): one resolution transition"
            );
            assert_eq!(
                settled.run_id, CHILD_RUN,
                "crash point {point} ({label}): ownership survives recovery"
            );
            assert_eq!(
                settled.source_handle,
                Some(format!("source:{CHILD}")),
                "crash point {point} ({label}): handle survives recovery"
            );
            // Reconciliation is a no-op on retry and leaves nothing unresolved.
            assert!(
                store
                    .reconcile_startup("reconcile:crash", EXPERIMENT)?
                    .is_empty(),
                "crash point {point} ({label}): reconciliation is idempotent"
            );
            assert!(
                store.reconciliation_candidates(EXPERIMENT)?.is_empty(),
                "crash point {point} ({label}): no unresolved candidate remains"
            );
            assert_eq!(
                orphan_rows(&database),
                OrphanRows::default(),
                "crash point {point} ({label}): recovery created no orphan row"
            );
        }
        // The crashed writer owned no destination the recovering owner cannot reclaim.
        assert!(
            leftover_sidecars(&database).is_empty(),
            "crash point {point} ({label}): crashed writer destinations reclaimed"
        );
        assert!(
            stray_entries(&directory).is_empty(),
            "crash point {point} ({label}): no unowned scratch destination remains"
        );
        remove_scratch(&directory);
    }
    Ok(())
}
