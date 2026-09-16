// SPDX-License-Identifier: MIT

//! Explicit cross-scope handle lookup refusal (issue #116 AC5).
//!
//! A checkpoint branch handle is only meaningful inside the scope that minted it. This suite
//! attempts to resolve a handle from a different project, run, episode, and agent/context scope and
//! asserts every attempt is refused, so a handle never aliases another scope's record:
//!
//! - the durable store's read surface is keyed by `(experiment_id, branch_id)`; a project scope
//!   that does not own the branch resolves nothing, and an unowned project refuses every mutation
//!   with `UnknownBranch`;
//! - a `run_id`, `episode_id`, or `context_id` is never a resolvable branch handle anywhere,
//!   including inside the scope that owns it; and
//! - the public projection handle is bound to the issuing key, the checkpoint identity, the exact
//!   state identity, and the occurrence, so another project/agent authority and another
//!   run/episode occurrence both refuse it, and a malformed handle is refused with `InvalidHandle`.
//!
//! The key-scope and state-scope halves of the projection are already covered by
//! `crates/harness/tests/checkpoint_projection.rs::a_different_key_cannot_test_candidate_states`
//! (line 89) and `::handles_are_deterministic_and_scoped` (line 69); the occurrence-scope axis and
//! the malformed-handle refusal are added here rather than repeated.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchFork, BranchStoreError,
    BranchStrategy, DurableBranchDraft, DurableBranchStatus, ExactCheckpointId, ExactStateDigest,
    HANDLE_PREFIX, MIN_HANDLE_KEY_BYTES, OccurrenceId, ProjectionError, ProjectionKey,
    SqliteBranchStore,
};

const ALPHA: &str = "project:alpha";
const BETA: &str = "project:beta";
const UNOWNED: &str = "project:gamma";
const ALPHA_ROOT: &str = "branch:alpha-root";
const BETA_ROOT: &str = "branch:beta-root";
const SHARED_ID: &str = "branch:alpha-child";
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

fn checkpoint(seed: char) -> ExactCheckpointId {
    ExactCheckpointId::parse(&format!(
        "asc-checkpoint:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("valid checkpoint id")
}

fn checkpoint_artifact() -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: format!("asc-checkpoint:v1:sha256:{}", "a".repeat(64)),
        role: BranchArtifactRole::Checkpoint,
    }
}

#[allow(clippy::too_many_arguments)]
fn draft(
    experiment_id: &str,
    root_branch_id: &str,
    branch_id: &str,
    parent_branch_id: Option<&str>,
    occurrence_id: &str,
    parent_occurrence_id: Option<&str>,
    run_id: &str,
    source_handle: &str,
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
        strategy: BranchStrategy::ExactRestore,
        source_handle: Some(source_handle.to_owned()),
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
        notes: Some("synthetic cross-scope attempt".to_owned()),
        artifacts: Vec::new(),
    }
}

fn alpha_store() -> Result<SqliteBranchStore, BranchStoreError> {
    let store = SqliteBranchStore::open_in_memory()?;
    store.create(
        "operation:alpha-root",
        draft(
            ALPHA,
            ALPHA_ROOT,
            ALPHA_ROOT,
            None,
            "occurrence:alpha-root",
            None,
            "run:alpha-root",
            "source:alpha-root",
        ),
    )?;
    store.create(
        "operation:alpha-child",
        draft(
            ALPHA,
            ALPHA_ROOT,
            SHARED_ID,
            Some(ALPHA_ROOT),
            "occurrence:alpha-child",
            Some("occurrence:alpha-root"),
            "run:alpha-child",
            "source:alpha-child",
        ),
    )?;
    // Another project may mint its own identity, including one that reuses the same literal
    // branch id; each scope only ever resolves its own record.
    store.create(
        "operation:beta-root",
        draft(
            BETA,
            BETA_ROOT,
            BETA_ROOT,
            None,
            "occurrence:beta-root",
            None,
            "run:beta-root",
            "source:beta-root",
        ),
    )?;
    store.create(
        "operation:beta-child",
        draft(
            BETA,
            BETA_ROOT,
            SHARED_ID,
            Some(BETA_ROOT),
            "occurrence:beta-child",
            Some("occurrence:beta-root"),
            "run:beta-child",
            "source:beta-child",
        ),
    )?;
    Ok(store)
}

#[test]
fn a_branch_handle_is_never_resolved_outside_its_project_scope() -> Result<(), BranchStoreError> {
    let store = alpha_store()?;
    let alpha = store
        .get(ALPHA, SHARED_ID)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(alpha.source_handle.as_deref(), Some("source:alpha-child"));
    assert_eq!(alpha.run_id, "run:alpha-child");
    assert_eq!(alpha.parent_branch_id.as_deref(), Some(ALPHA_ROOT));
    // The same literal branch id in another project resolves that project's record, never alpha's.
    let beta = store
        .get(BETA, SHARED_ID)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(beta.source_handle.as_deref(), Some("source:beta-child"));
    assert_eq!(beta.run_id, "run:beta-child");
    assert_eq!(beta.parent_branch_id.as_deref(), Some(BETA_ROOT));
    assert_ne!(alpha.source_handle, beta.source_handle);
    // A scope that owns no such branch resolves nothing.
    assert!(
        store.get(UNOWNED, SHARED_ID)?.is_none(),
        "an unowned project scope resolves no handle"
    );
    assert!(
        store.get(ALPHA, BETA_ROOT)?.is_none(),
        "a sibling project's branch id is not resolvable"
    );
    assert!(
        store.list(UNOWNED, None, MAX_PAGE)?.branches.is_empty(),
        "an unowned project scope lists nothing"
    );
    // Every read and mutation through an unowned scope fails closed with the owning error.
    assert_eq!(
        store.ancestry(UNOWNED, SHARED_ID),
        Err(BranchStoreError::UnknownBranch)
    );
    assert_eq!(
        store.transition(
            "operation:cross-transition",
            UNOWNED,
            SHARED_ID,
            0,
            DurableBranchStatus::Archived
        ),
        Err(BranchStoreError::UnknownBranch)
    );
    assert_eq!(
        store.set_assurance(
            "operation:cross-assurance",
            UNOWNED,
            SHARED_ID,
            0,
            BranchAssurance::ExactRestoreReceipt
        ),
        Err(BranchStoreError::UnknownBranch)
    );
    assert_eq!(
        store.attach_artifact(
            "operation:cross-artifact",
            UNOWNED,
            SHARED_ID,
            0,
            checkpoint_artifact()
        ),
        Err(BranchStoreError::UnknownBranch)
    );
    assert_eq!(
        store.rename("operation:cross-rename", UNOWNED, SHARED_ID, 0, "cross"),
        Err(BranchStoreError::UnknownBranch)
    );
    // No event or reconciliation candidate leaks across the scope boundary.
    let events = store.events(UNOWNED, 0, MAX_PAGE)?;
    assert!(events.events.is_empty());
    assert_eq!(events.oldest_sequence, None);
    assert!(
        store.reconciliation_candidates(UNOWNED)?.is_empty(),
        "an unowned scope has no reconciliation candidate"
    );
    assert!(
        store
            .reconcile_startup("reconcile:cross", UNOWNED)?
            .is_empty(),
        "an unowned scope resolves no branch to reconcile"
    );
    let beta_events = store.events(BETA, 0, MAX_PAGE)?;
    assert!(
        beta_events
            .events
            .iter()
            .all(|event| event.experiment_id == BETA && !event.operation_id.contains("alpha")),
        "another scope's event log never exposes this scope's operation"
    );
    // The owning scope is unaffected by the refused attempts.
    let settled = store
        .get(ALPHA, SHARED_ID)?
        .ok_or(BranchStoreError::UnknownBranch)?;
    assert_eq!(settled.metadata_revision, alpha.metadata_revision);
    assert_eq!(settled.status, DurableBranchStatus::Pending);
    Ok(())
}

#[test]
fn run_episode_and_context_handles_are_not_branch_handles() -> Result<(), BranchStoreError> {
    let store = alpha_store()?;
    // A run, episode, or agent-context handle never resolves as a branch handle, not even inside
    // the scope that recorded it.
    for scope in [ALPHA, BETA] {
        for alias in [
            "run:alpha-child",
            "episode:branch:alpha-child",
            "context:branch:alpha-child",
            "trajectory:branch:alpha-child",
        ] {
            assert!(
                store.get(scope, alias)?.is_none(),
                "handle {alias} must not resolve as a branch handle in {scope}"
            );
        }
    }
    // The only lookup key is the immutable `(project, branch)` pair, so the same run identity in
    // another project cannot be used to reach this project's branch either.
    assert!(
        store
            .ancestry(BETA, "run:alpha-child")
            .is_err_and(|error| error == BranchStoreError::UnknownBranch),
        "another scope cannot walk ancestry from a run handle"
    );
    Ok(())
}

#[test]
fn a_public_projection_handle_is_refused_outside_its_occurrence_scope() {
    let trusted = ProjectionKey::new(&[9; MIN_HANDLE_KEY_BYTES]).expect("strong key");
    let handle = trusted
        .handle(&checkpoint('b'), &state('a'), &occurrence("run:one"))
        .expect("handle issues");
    assert!(handle.starts_with(HANDLE_PREFIX));
    assert!(
        trusted
            .matches(
                &handle,
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect("the issuing scope resolves its own handle")
    );
    // Another run or episode occurrence in the same project refuses the handle.
    assert!(
        !trusted
            .matches(
                &handle,
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:two")
            )
            .expect("comparison runs")
    );
    // Another project's checkpoint identity refuses the handle even for the same occurrence.
    assert!(
        !trusted
            .matches(
                &handle,
                &checkpoint('c'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect("comparison runs")
    );
    // Every other project/agent authority refuses the handle.
    for seed in [1_u8, 2, 3, 4] {
        let other = ProjectionKey::new(&[seed; MIN_HANDLE_KEY_BYTES]).expect("strong key");
        assert!(
            !other
                .matches(
                    &handle,
                    &checkpoint('b'),
                    &state('a'),
                    &occurrence("run:one")
                )
                .expect("comparison runs"),
            "projection key {seed} must not resolve another authority's handle"
        );
    }
    // A malformed handle is refused with a specific error rather than resolving.
    assert_eq!(
        trusted
            .matches(
                "ckpt-h1:zz",
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .unwrap_err(),
        ProjectionError::InvalidHandle
    );
    assert_eq!(
        trusted
            .matches(
                "handle:without:prefix",
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .unwrap_err(),
        ProjectionError::InvalidHandle
    );
}
