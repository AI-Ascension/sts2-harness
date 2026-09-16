// SPDX-License-Identifier: MIT

//! Fixtures shared by the durable checkpoint-branch crash-point suites.
//!
//! Each crash suite is its own integration binary, so the helpers they agree on live here: the
//! drafts that describe the synthetic ownership edges, the scratch directory that holds a crashed
//! writer's store file, and the raw-SQL readers that show no retained row outlives the
//! destination that owned it.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchFork, BranchStrategy,
    DurableBranchDraft, DurableBranchStatus, ExactStateDigest, OccurrenceId,
};

pub const EXPERIMENT: &str = "experiment:crash";
pub const ROOT: &str = "branch:crash-root";
pub const ROOT_RUN: &str = "run:crash-root";
pub const ROOT_OCCURRENCE: &str = "occurrence:crash-root";
pub const CHILD: &str = "branch:crash-child";
pub const CHILD_RUN: &str = "run:crash-child";
pub const CHILD_OCCURRENCE: &str = "occurrence:crash-child";
pub const MAX_PAGE: u64 = 128;

/// The four persistence crash points, in commit order.
pub const CRASH_POINTS: [(u8, &str); 4] = [
    (1, "fork intent"),
    (2, "artifact association"),
    (3, "strategy completion"),
    (4, "ready publication"),
];

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

pub fn checkpoint_artifact() -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: format!("asc-checkpoint:v1:sha256:{}", "a".repeat(64)),
        role: BranchArtifactRole::Checkpoint,
    }
}

pub fn draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    occurrence_id: &str,
    parent_occurrence_id: Option<&str>,
    run_id: &str,
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
        run_id: run_id.to_owned(),
        episode_id: Some(format!("episode:{branch_id}")),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: "policy:v1".to_owned(),
        config_revision: "config:v1".to_owned(),
        name: branch_id.to_owned(),
        notes: Some("synthetic crash-point branch".to_owned()),
        artifacts: Vec::new(),
    }
}

pub fn root_draft() -> DurableBranchDraft {
    draft(ROOT, None, ROOT_OCCURRENCE, None, ROOT_RUN)
}

pub fn child_draft() -> DurableBranchDraft {
    draft(
        CHILD,
        Some(ROOT),
        CHILD_OCCURRENCE,
        Some(ROOT_OCCURRENCE),
        CHILD_RUN,
    )
}

pub fn scratch_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-branch-crash-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("scratch directory is creatable");
    path
}

pub fn remove_scratch(directory: &Path) {
    fs::remove_dir_all(directory).expect("scratch directory is removable");
}

/// The state the crashing writer committed at each crash point.
pub fn committed_state(point: u8) -> (DurableBranchStatus, u64, usize) {
    match point {
        1 => (DurableBranchStatus::Pending, 0, 0),
        2 => (DurableBranchStatus::Pending, 1, 1),
        3 => (DurableBranchStatus::Restoring, 3, 1),
        _ => (DurableBranchStatus::Ready, 4, 1),
    }
}

/// The deterministic reconciliation resolution for the half-created child.
pub fn reconciled_status(point: u8) -> DurableBranchStatus {
    match point {
        1 | 2 => DurableBranchStatus::Archived,
        3 => DurableBranchStatus::Failed,
        _ => DurableBranchStatus::Ready,
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct OrphanRows {
    pub artifact_edges: i64,
    pub operation_rows: i64,
    pub detached_branches: i64,
}

/// Counts retained rows whose owning destination no longer exists.
pub fn orphan_rows(database: &Path) -> OrphanRows {
    let connection = Connection::open(database).expect("raw connection opens");
    let count = |sql: &str| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("orphan count runs")
    };
    OrphanRows {
        artifact_edges: count(ARTIFACT_ORPHANS),
        operation_rows: count(OPERATION_ORPHANS),
        detached_branches: count(DETACHED_BRANCHES),
    }
}

const ARTIFACT_ORPHANS: &str = "SELECT COUNT(*) FROM branch_artifacts a \
     WHERE NOT EXISTS (SELECT 1 FROM branch_operations o WHERE o.operation_id = a.operation_id)";
const OPERATION_ORPHANS: &str = "SELECT COUNT(*) FROM branch_operations o \
     WHERE NOT EXISTS (SELECT 1 FROM durable_branches b \
     WHERE b.experiment_id = o.experiment_id AND b.branch_id = o.branch_id)";
const DETACHED_BRANCHES: &str = "SELECT COUNT(*) FROM durable_branches b \
     WHERE b.parent_branch_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM durable_branches p \
     WHERE p.experiment_id = b.experiment_id AND p.branch_id = b.parent_branch_id)";

/// Counts rows of one experiment-scoped table.
pub fn table_rows(database: &Path, table: &str) -> i64 {
    let connection = Connection::open(database).expect("raw connection opens");
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE experiment_id = ?1"),
            [EXPERIMENT],
            |row| row.get::<_, i64>(0),
        )
        .expect("row count runs")
}

/// Lists the writer destinations a crashed process can leave beside the store file.
pub fn leftover_sidecars(database: &Path) -> Vec<String> {
    let mut leftovers = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", database.display()));
        if sidecar.exists() {
            leftovers.push(format!("branches.sqlite3{suffix}"));
        }
    }
    leftovers
}

/// Lists every scratch entry that is not the owned store file.
pub fn stray_entries(directory: &Path) -> Vec<String> {
    let mut stray = Vec::new();
    for entry in fs::read_dir(directory).expect("scratch directory is readable") {
        let name = entry
            .expect("directory entry is readable")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name != "branches.sqlite3" {
            stray.push(name);
        }
    }
    stray
}
