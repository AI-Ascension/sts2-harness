// SPDX-License-Identifier: MIT

//! Issue #116 AC6 clause (d): integration consumers round-trip both strategy descriptors.
//!
//! The production consumer is the runtime binary's startup reconciliation in
//! `src/bin/runtime_support/continuation_branches.rs`: it constructs the durable branch store,
//! resolves every half-created continuation branch for its experiment scope, and reads each
//! resolved branch back to confirm the strategy descriptor survived. Both tests below drive that
//! exact production source: the first compiles it into this suite, the second observes the real
//! `sts2-harness-runtime` process reconciling a pre-seeded store.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BranchAssurance, BranchFork, BranchStrategy, DurableBranch, DurableBranchDraft,
    DurableBranchStatus, EXO_SOURCE_REVISION, ExactStateDigest, OccurrenceId, SqliteBranchStore,
};

#[path = "../src/bin/runtime_support/continuation_branches.rs"]
mod continuation_branches;

#[path = "support/completed_resume_process_support.rs"]
// Only the timeout-overriding runner is used here; the shared module also carries helpers for
// sibling suites.
#[allow(dead_code)]
mod process_support;

use continuation_branches::{
    STARTUP_RECONCILE_OPERATION_PREFIX, descriptor_preserved, reconcile_continuation_branches,
};
use process_support::run_child_with_timeout;

/// The refusal path still runs the bounded telemetry export, so allow slack beyond the 5s default.
const CHILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const EPISODE_ID: &str = "episode-durable-branch-consumer";
const EXPERIMENT_ID: &str = "experiment:episode-durable-branch-consumer";
const ROOT_BRANCH: &str = "branch:root";
const EXACT_BRANCH: &str = "branch:exact-restore";
const PREFIX_BRANCH: &str = "branch:prefix-replay";

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
    let exact = strategy == BranchStrategy::ExactRestore;
    DurableBranchDraft {
        experiment_id: EXPERIMENT_ID.to_owned(),
        root_branch_id: ROOT_BRANCH.to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(&format!("occurrence:{branch_id}")),
            parent_occurrence_id: parent_branch_id.map(|_| occurrence("occurrence:branch:root")),
            state_digest: state(if exact { 'a' } else { 'b' }),
        },
        strategy,
        source_handle: exact.then(|| format!("checkpoint:{branch_id}")),
        trajectory_prefix: (!exact).then(|| format!("trajectory-prefix:{branch_id}")),
        effective_seed: Some("seed:durable-consumer".to_owned()),
        setup_digest: Some(format!("setup:{branch_id}")),
        boundary: String::from(if exact { "decision" } else { "observation" }),
        assurance: BranchAssurance::Unverified,
        run_id: format!("run:{branch_id}"),
        episode_id: Some(EPISODE_ID.to_owned()),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: "policy:durable-consumer".to_owned(),
        config_revision: "config:durable-consumer".to_owned(),
        name: branch_id.to_owned(),
        notes: Some("production continuation round trip".to_owned()),
        artifacts: Vec::new(),
    }
}

fn fixture_root() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before epoch: {error}"))?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sts2-durable-branch-consumer-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root).map_err(|error| format!("cannot create fixture directory: {error}"))?;
    Ok(root)
}

/// Persists one ready root plus one half-created branch per strategy, then closes the store.
///
/// The returned records are the pre-restart snapshot: a ready `exact_restore` root carrying its
/// native receipt, a `pending` exact-restore child, and a `replaying` prefix-replay child. Dropping
/// the handle closes the database, so the consumer below reopens the persisted file.
fn seed_branches(store_path: &Path) -> Result<Vec<DurableBranch>, String> {
    let store = SqliteBranchStore::open(store_path).map_err(|error| error.to_string())?;
    let mut root = draft(ROOT_BRANCH, None, BranchStrategy::ExactRestore);
    root.fork.parent_occurrence_id = None;
    let root = store
        .create("operation:root", root)
        .map_err(|error| error.to_string())?;
    let root = store
        .transition(
            "operation:root-restoring",
            EXPERIMENT_ID,
            ROOT_BRANCH,
            root.metadata_revision,
            DurableBranchStatus::Restoring,
        )
        .map_err(|error| error.to_string())?;
    // Readiness evidence is recorded on a started strategy and is reset by later transitions into
    // `restoring`/`replaying`, so the receipt is attached immediately before `ready`.
    let root = store
        .set_assurance(
            "operation:root-receipt",
            EXPERIMENT_ID,
            ROOT_BRANCH,
            root.metadata_revision,
            BranchAssurance::ExactRestoreReceipt,
        )
        .map_err(|error| error.to_string())?;
    let root = store
        .transition(
            "operation:root-ready",
            EXPERIMENT_ID,
            ROOT_BRANCH,
            root.metadata_revision,
            DurableBranchStatus::Ready,
        )
        .map_err(|error| error.to_string())?;
    let exact = store
        .create(
            "operation:exact",
            draft(
                EXACT_BRANCH,
                Some(ROOT_BRANCH),
                BranchStrategy::ExactRestore,
            ),
        )
        .map_err(|error| error.to_string())?;
    let prefix = store
        .create(
            "operation:prefix",
            draft(
                PREFIX_BRANCH,
                Some(ROOT_BRANCH),
                BranchStrategy::PrefixReplay,
            ),
        )
        .map_err(|error| error.to_string())?;
    let prefix = store
        .transition(
            "operation:prefix-replaying",
            EXPERIMENT_ID,
            PREFIX_BRANCH,
            prefix.metadata_revision,
            DurableBranchStatus::Replaying,
        )
        .map_err(|error| error.to_string())?;
    Ok(vec![root, exact, prefix])
}

fn read_branch(store_path: &Path, branch_id: &str) -> Result<DurableBranch, String> {
    SqliteBranchStore::open(store_path)
        .map_err(|error| error.to_string())?
        .get(EXPERIMENT_ID, branch_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("continuation branch {branch_id} is missing from the store"))
}

fn find<'a>(branches: &'a [DurableBranch], branch_id: &str) -> Result<&'a DurableBranch, String> {
    branches
        .iter()
        .find(|branch| branch.branch_id == branch_id)
        .ok_or_else(|| format!("continuation branch {branch_id} is absent"))
}

fn assert_seeded(before: &[DurableBranch]) -> Result<(), String> {
    for (branch_id, expected) in [
        (ROOT_BRANCH, DurableBranchStatus::Ready),
        (EXACT_BRANCH, DurableBranchStatus::Pending),
        (PREFIX_BRANCH, DurableBranchStatus::Replaying),
    ] {
        let actual = find(before, branch_id)?.status;
        if actual != expected {
            return Err(format!(
                "{branch_id} was seeded as {actual:?}, expected {expected:?}"
            ));
        }
    }
    Ok(())
}

/// Asserts both half-created branches were resolved as documented with their descriptor intact.
fn assert_resolved(before: &[DurableBranch], resolved: &[DurableBranch]) -> Result<(), String> {
    for (branch_id, expected_status) in [
        (EXACT_BRANCH, DurableBranchStatus::Archived),
        (PREFIX_BRANCH, DurableBranchStatus::Failed),
    ] {
        let expected = find(before, branch_id)?;
        let persisted = find(resolved, branch_id)?;
        if !descriptor_preserved(expected, persisted) {
            return Err(format!(
                "{branch_id} lost its continuation identity or strategy descriptor"
            ));
        }
        if persisted.status != expected_status {
            return Err(format!(
                "{branch_id} was reconciled to {:?}, expected {expected_status:?}",
                persisted.status
            ));
        }
    }
    Ok(())
}

/// Asserts a fully created continuation branch was not disturbed by startup reconciliation.
fn assert_untouched(before: &[DurableBranch], root_branch: &DurableBranch) -> Result<(), String> {
    let expected = find(before, ROOT_BRANCH)?;
    if !descriptor_preserved(expected, root_branch) {
        return Err(String::from("the ready continuation branch changed"));
    }
    if root_branch.status != DurableBranchStatus::Ready
        || root_branch.metadata_revision != expected.metadata_revision
    {
        return Err(String::from(
            "reconciliation mutated a fully created continuation branch",
        ));
    }
    Ok(())
}

#[test]
fn production_consumer_round_trips_both_strategy_descriptors() -> Result<(), String> {
    let root = fixture_root()?;
    let store_path = root.join("continuation-branches.sqlite3");
    let before = seed_branches(&store_path)?;
    assert_seeded(&before)?;

    // Restart: the seeding handle is closed, so this reopens the persisted file the way the runtime
    // binary does on process start, then runs the production startup reconciliation.
    let reconciled = reconcile_continuation_branches(
        &store_path,
        STARTUP_RECONCILE_OPERATION_PREFIX,
        EXPERIMENT_ID,
    )?;
    assert_resolved(&before, &reconciled)?;

    let exact = read_branch(&store_path, EXACT_BRANCH)?;
    let prefix = read_branch(&store_path, PREFIX_BRANCH)?;
    if !descriptor_preserved(find(&before, EXACT_BRANCH)?, &exact)
        || !descriptor_preserved(find(&before, PREFIX_BRANCH)?, &prefix)
    {
        return Err(String::from(
            "a persisted strategy descriptor changed across reconciliation",
        ));
    }
    if exact.strategy != BranchStrategy::ExactRestore
        || exact.source_handle.is_none()
        || exact.trajectory_prefix.is_some()
    {
        return Err(String::from("the exact-restore descriptor is not intact"));
    }
    if prefix.strategy != BranchStrategy::PrefixReplay
        || prefix.trajectory_prefix.is_none()
        || prefix.source_handle.is_some()
    {
        return Err(String::from("the prefix-replay descriptor is not intact"));
    }
    if exact.fork.state_digest == prefix.fork.state_digest {
        return Err(String::from(
            "the two strategy descriptors collapsed onto one state digest",
        ));
    }
    let root_branch = read_branch(&store_path, ROOT_BRANCH)?;
    assert_untouched(&before, &root_branch)?;

    // Reconciliation is idempotent: a retried startup resolves nothing a second time.
    let retried = reconcile_continuation_branches(
        &store_path,
        STARTUP_RECONCILE_OPERATION_PREFIX,
        EXPERIMENT_ID,
    )?;
    if !retried.is_empty() {
        return Err(String::from(
            "startup reconciliation resolved branches twice",
        ));
    }
    Ok(())
}

fn runtime_command(store_path: &Path, execution_store: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
    command
        .env_clear()
        .arg("--resume")
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
        .env("STS2_GATEWAY_ADDR", "127.0.0.1:1")
        .env("STS2_GATEWAY_TOKEN", "test-token")
        .env("STS2_MCP_BINARY", "/bin/true")
        .env("STS2_INSTANCE_ID", "instance-durable-branch-consumer")
        .env("STS2_CALLER_ID", "caller-durable-branch-consumer")
        .env("STS2_SESSION_ID", "session-durable-branch-consumer")
        .env("STS2_LEASE_ID", "lease-durable-branch-consumer")
        .env("STS2_LEASE_EPOCH", "1")
        .env("STS2_MCP_SESSION_ID", "mcp-durable-branch-consumer")
        .env("STS2_RUN_ID", "run-durable-branch-consumer")
        .env("STS2_EPISODE_ID", EPISODE_ID)
        .env("STS2_ATTEMPT_ID", "attempt-durable-branch-consumer")
        .env("STS2_TRAJECTORY_ID", "trajectory-durable-branch-consumer")
        .env("STS2_TRACE_ID", "trace-durable-branch-consumer")
        .env("STS2_ARTIFACT_ID", "artifact-durable-branch-consumer")
        .env("STS2_EXECUTION_STORE_PATH", execution_store)
        .env("STS2_BRANCH_STORE_PATH", store_path)
        .env("STS2_SEED", "seed-durable-branch-consumer")
        .env("STS2_BUILD_DIGEST", "build-durable-branch-consumer")
        .env("STS2_STATE_DIGEST", "state-durable-branch-consumer")
        .env("STS2_EXO_REVISION", EXO_SOURCE_REVISION)
        .env("STS2_EXO_ADMISSION", "legacy")
        .env("STS2_EXO_BRIDGE_BINARY", "/bin/true")
        .env("STS2_EXO_FORWARD_VISIBLE_SEED", "true")
        .env("STS2_OBJECTIVE", "complete the round trip");
    command
}

#[test]
fn production_runtime_process_reconciles_seeded_strategy_descriptors() -> Result<(), String> {
    let root = fixture_root()?;
    let store_path = root.join("continuation-branches.sqlite3");
    let execution_store = root.join("execution.sqlite3");
    let before = seed_branches(&store_path)?;
    assert_seeded(&before)?;
    if execution_store.exists() {
        return Err(String::from("the episode store must not pre-exist"));
    }

    let output = run_child_with_timeout(
        runtime_command(&store_path, &execution_store),
        CHILD_TIMEOUT,
    )?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() != Some(2) {
        return Err(format!(
            "runtime child exited with {:?}: {stderr}",
            output.status.code()
        ));
    }
    // The child refused to admit the episode; startup reconciliation runs before that admission.
    if !stderr.contains("resume requested but no durable episode exists") {
        return Err(format!(
            "runtime child did not reach continuation admission: {stderr}"
        ));
    }

    // Only the production process touched the store after seeding, so every resolved branch below
    // is evidence that the real binary constructed the store and ran startup reconciliation.
    let after = vec![
        read_branch(&store_path, ROOT_BRANCH)?,
        read_branch(&store_path, EXACT_BRANCH)?,
        read_branch(&store_path, PREFIX_BRANCH)?,
    ];
    assert_resolved(&before, &after)?;
    assert_untouched(&before, find(&after, ROOT_BRANCH)?)
}
