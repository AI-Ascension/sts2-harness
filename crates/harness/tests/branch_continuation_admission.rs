// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sts2_harness::{
    BranchArtifactReference, BranchArtifactResolver, BranchArtifactRole, BranchArtifactState,
    BranchAssurance, BranchContinuationAdmissionError, BranchContinuationSelector,
    BranchContinuationStrategyPlan, BranchFork, BranchStoreError, BranchStrategy,
    DurableBranchDraft, DurableBranchStatus, ExactStateDigest, OccurrenceId, SqliteBranchStore,
    admit_branch_continuation, admit_running_branch_continuation,
};

const EXPERIMENT: &str = "experiment:continuation-admission";
const ROOT: &str = "branch:root";

fn state() -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "a".repeat(64)))
        .expect("valid state digest")
}

fn occurrence(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("valid occurrence")
}

fn artifact(artifact_id: &str, role: BranchArtifactRole) -> BranchArtifactReference {
    BranchArtifactReference {
        artifact_id: artifact_id.to_owned(),
        role,
    }
}

fn draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    strategy: BranchStrategy,
    artifacts: Vec<BranchArtifactReference>,
) -> DurableBranchDraft {
    let is_exact = strategy == BranchStrategy::ExactRestore;
    DurableBranchDraft {
        experiment_id: EXPERIMENT.to_owned(),
        root_branch_id: ROOT.to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: occurrence(&format!("occurrence:{branch_id}")),
            parent_occurrence_id: parent_branch_id.map(|_| occurrence("occurrence:branch:root")),
            state_digest: state(),
        },
        strategy,
        source_handle: is_exact.then(|| format!("checkpoint-handle:{branch_id}")),
        trajectory_prefix: (!is_exact).then(|| format!("trajectory-prefix:{branch_id}")),
        effective_seed: Some("seed:continuation-admission".to_owned()),
        setup_digest: Some("setup:continuation-admission".to_owned()),
        boundary: "decision".to_owned(),
        assurance: BranchAssurance::Unverified,
        run_id: format!("run:{branch_id}"),
        episode_id: Some(format!("episode:{branch_id}")),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: "policy:continuation-admission".to_owned(),
        config_revision: "config:continuation-admission".to_owned(),
        name: branch_id.to_owned(),
        notes: None,
        artifacts,
    }
}

fn create_root(store: &SqliteBranchStore) -> Result<(), BranchStoreError> {
    store.create(
        "operation:create-root",
        draft(ROOT, None, BranchStrategy::ExactRestore, Vec::new()),
    )?;
    Ok(())
}

fn create_ready_branch(
    store: &SqliteBranchStore,
    branch_id: &str,
    strategy: BranchStrategy,
    artifacts: Vec<BranchArtifactReference>,
) -> Result<(), BranchStoreError> {
    let branch = store.create(
        &format!("operation:create:{branch_id}"),
        draft(branch_id, Some(ROOT), strategy, artifacts),
    )?;
    let preparing = match strategy {
        BranchStrategy::ExactRestore => DurableBranchStatus::Restoring,
        BranchStrategy::PrefixReplay => DurableBranchStatus::Replaying,
    };
    let branch = store.transition(
        &format!("operation:prepare:{branch_id}"),
        EXPERIMENT,
        branch_id,
        branch.metadata_revision,
        preparing,
    )?;
    let assurance = match strategy {
        BranchStrategy::ExactRestore => BranchAssurance::ExactRestoreReceipt,
        BranchStrategy::PrefixReplay => BranchAssurance::PrefixReplayBoundary,
    };
    let branch = store.set_assurance(
        &format!("operation:assure:{branch_id}"),
        EXPERIMENT,
        branch_id,
        branch.metadata_revision,
        assurance,
    )?;
    store.transition(
        &format!("operation:ready:{branch_id}"),
        EXPERIMENT,
        branch_id,
        branch.metadata_revision,
        DurableBranchStatus::Ready,
    )?;
    Ok(())
}

#[derive(Default)]
struct FixtureResolver {
    states: BTreeMap<String, BranchArtifactState>,
}

impl BranchArtifactResolver for FixtureResolver {
    fn resolve(&self, artifact_id: &str) -> BranchArtifactState {
        self.states
            .get(artifact_id)
            .copied()
            .unwrap_or(BranchArtifactState::Unverifiable)
    }
}

#[test]
fn exact_restore_selection_returns_its_scoped_restore_plan()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    let checkpoint = artifact("artifact:checkpoint:exact", BranchArtifactRole::Checkpoint);
    let closure = artifact(
        "artifact:restore-closure:exact",
        BranchArtifactRole::RestoreClosure,
    );
    let context = artifact(
        "artifact:context:exact",
        BranchArtifactRole::ContextSnapshot,
    );
    create_ready_branch(
        &store,
        "branch:exact",
        BranchStrategy::ExactRestore,
        vec![checkpoint.clone(), closure.clone(), context.clone()],
    )?;
    let resolver = FixtureResolver {
        states: [
            (
                checkpoint.artifact_id.clone(),
                BranchArtifactState::Available,
            ),
            (closure.artifact_id.clone(), BranchArtifactState::Available),
            (context.artifact_id.clone(), BranchArtifactState::Available),
        ]
        .into(),
    };
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:exact")?;

    let admission = admit_branch_continuation(&store, &selector, &resolver)?;

    assert_eq!(admission.branch.experiment_id, EXPERIMENT);
    assert_eq!(admission.branch.branch_id, "branch:exact");
    assert_eq!(admission.branch.run_id, "run:branch:exact");
    assert_eq!(
        admission.branch.episode_id.as_deref(),
        Some("episode:branch:exact")
    );
    assert_eq!(
        admission.branch.trajectory_id.as_deref(),
        Some("trajectory:branch:exact")
    );
    assert_eq!(
        admission.branch.context_id.as_deref(),
        Some("context:branch:exact")
    );
    assert_eq!(admission.branch.status, DurableBranchStatus::Ready);
    assert_eq!(
        admission.branch.assurance,
        BranchAssurance::ExactRestoreReceipt
    );
    assert_eq!(
        admission.strategy,
        BranchContinuationStrategyPlan::ExactRestore {
            checkpoint,
            restore_closure: vec![closure],
        }
    );
    assert!(admission.artifacts.all_available());
    Ok(())
}

#[test]
fn running_exact_restore_requires_a_retained_destination_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    let checkpoint = artifact(
        "artifact:checkpoint:running",
        BranchArtifactRole::Checkpoint,
    );
    create_ready_branch(
        &store,
        "branch:running-exact",
        BranchStrategy::ExactRestore,
        vec![checkpoint.clone()],
    )?;
    let ready = store
        .get(EXPERIMENT, "branch:running-exact")?
        .expect("running exact fixture");
    store.transition(
        "operation:running-exact",
        EXPERIMENT,
        "branch:running-exact",
        ready.metadata_revision,
        DurableBranchStatus::Running,
    )?;
    let resolver = FixtureResolver {
        states: [(checkpoint.artifact_id, BranchArtifactState::Available)].into(),
    };
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:running-exact")?;
    let error = admit_running_branch_continuation(&store, &selector, &resolver)
        .expect_err("running exact branch without receipt must be refused");
    assert!(matches!(
        error,
        BranchContinuationAdmissionError::MissingStrategyArtifact {
            role: BranchArtifactRole::ContextSnapshot
        }
    ));
    Ok(())
}

#[test]
fn prefix_replay_selection_keeps_its_weaker_assurance_and_source()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    let replay_prefix = artifact(
        "artifact:replay-prefix:prefix",
        BranchArtifactRole::ReplayPrefix,
    );
    create_ready_branch(
        &store,
        "branch:prefix",
        BranchStrategy::PrefixReplay,
        vec![replay_prefix.clone()],
    )?;
    let resolver = FixtureResolver {
        states: [(
            replay_prefix.artifact_id.clone(),
            BranchArtifactState::Available,
        )]
        .into(),
    };
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:prefix")?;

    let admission = admit_branch_continuation(&store, &selector, &resolver)?;

    assert_eq!(
        admission.branch.assurance,
        BranchAssurance::PrefixReplayBoundary
    );
    assert_eq!(
        admission.branch.trajectory_prefix.as_deref(),
        Some("trajectory-prefix:branch:prefix")
    );
    assert_eq!(
        admission.strategy,
        BranchContinuationStrategyPlan::PrefixReplay { replay_prefix }
    );
    assert!(admission.artifacts.all_available());
    Ok(())
}

#[test]
fn pending_branch_and_unknown_scoped_branch_are_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    let selector = BranchContinuationSelector::new(EXPERIMENT, ROOT)?;
    let error = admit_branch_continuation(&store, &selector, &FixtureResolver::default())
        .expect_err("pending root cannot be selected");
    assert_eq!(
        error,
        BranchContinuationAdmissionError::BranchNotReady {
            status: DurableBranchStatus::Pending,
        }
    );

    let wrong_scope = BranchContinuationSelector::new("experiment:other", ROOT)?;
    let error = admit_branch_continuation(&store, &wrong_scope, &FixtureResolver::default())
        .expect_err("branch lookup must remain scoped to its experiment");
    assert_eq!(
        error,
        BranchContinuationAdmissionError::Store(BranchStoreError::UnknownBranch)
    );
    Ok(())
}

#[test]
fn ready_branch_without_its_strategy_artifact_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    create_ready_branch(
        &store,
        "branch:missing-checkpoint",
        BranchStrategy::ExactRestore,
        Vec::new(),
    )?;
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:missing-checkpoint")?;

    let error = admit_branch_continuation(&store, &selector, &FixtureResolver::default())
        .expect_err("empty retention is not sufficient for exact restore");

    assert_eq!(
        error,
        BranchContinuationAdmissionError::MissingStrategyArtifact {
            role: BranchArtifactRole::Checkpoint,
        }
    );
    Ok(())
}

#[test]
fn ambiguous_strategy_source_artifacts_are_refused() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    create_ready_branch(
        &store,
        "branch:ambiguous",
        BranchStrategy::ExactRestore,
        vec![
            artifact("artifact:checkpoint:first", BranchArtifactRole::Checkpoint),
            artifact("artifact:checkpoint:second", BranchArtifactRole::Checkpoint),
        ],
    )?;
    let resolver = FixtureResolver {
        states: [
            (
                "artifact:checkpoint:first".to_owned(),
                BranchArtifactState::Available,
            ),
            (
                "artifact:checkpoint:second".to_owned(),
                BranchArtifactState::Available,
            ),
        ]
        .into(),
    };
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:ambiguous")?;

    let error = admit_branch_continuation(&store, &selector, &resolver)
        .expect_err("ambiguous exact sources must not select one arbitrarily");

    assert_eq!(
        error,
        BranchContinuationAdmissionError::AmbiguousStrategyArtifact {
            role: BranchArtifactRole::Checkpoint,
        }
    );
    Ok(())
}

#[test]
fn missing_or_unverifiable_retained_artifacts_block_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open_in_memory()?;
    create_root(&store)?;
    for (branch_id, artifact_state) in [
        ("branch:missing", BranchArtifactState::Missing),
        ("branch:unverifiable", BranchArtifactState::Unverifiable),
    ] {
        let checkpoint = artifact(
            &format!("artifact:checkpoint:{branch_id}"),
            BranchArtifactRole::Checkpoint,
        );
        create_ready_branch(
            &store,
            branch_id,
            BranchStrategy::ExactRestore,
            vec![checkpoint.clone()],
        )?;
        let resolver = FixtureResolver {
            states: [(checkpoint.artifact_id.clone(), artifact_state)].into(),
        };
        let selector = BranchContinuationSelector::new(EXPERIMENT, branch_id)?;

        let error = admit_branch_continuation(&store, &selector, &resolver)
            .expect_err("unavailable checkpoint must block admission");

        let BranchContinuationAdmissionError::ArtifactsUnavailable(availability) = error else {
            return Err("unexpected admission error".into());
        };
        assert_eq!(availability.unavailable().len(), 1);
        assert_eq!(availability.unavailable()[0].state, artifact_state);
    }
    Ok(())
}

#[test]
fn selector_rejects_unbounded_or_empty_namespaces() {
    assert_eq!(
        BranchContinuationSelector::new("", "branch:one"),
        Err(BranchContinuationAdmissionError::InvalidSelector)
    );
    assert_eq!(
        BranchContinuationSelector::new(EXPERIMENT, format!("branch:{}", "x".repeat(300))),
        Err(BranchContinuationAdmissionError::InvalidSelector)
    );
}
