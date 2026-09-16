// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sts2_harness::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchContinuationSelector,
    BranchContinuationStrategyPlan, BranchFork, BranchStrategy, DurableBranchDraft,
    DurableBranchStatus, ExactArtifactStore, ExactStateDigest, OccurrenceId, SqliteBranchStore,
};

use super::{
    BranchContinuationEffectPort, SelectedBranchContinuation, bind_branch_identities, dispatch,
};

const EXPERIMENT: &str = "experiment:runtime-continuation";
const ROOT: &str = "branch:root";
static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> std::io::Result<Self> {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sts2-branch-continuation-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state() -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "b".repeat(64)))
        .expect("valid exact state digest")
}

fn create_store(
    path: &Path,
    artifacts: &ExactArtifactStore,
    strategy: BranchStrategy,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteBranchStore::open(path)?;
    let root = draft(ROOT, None, BranchStrategy::ExactRestore, Vec::new());
    store.create("operation:create-root", root)?;

    let role = match strategy {
        BranchStrategy::ExactRestore => BranchArtifactRole::Checkpoint,
        BranchStrategy::PrefixReplay => BranchArtifactRole::ReplayPrefix,
    };
    let bytes = match strategy {
        BranchStrategy::ExactRestore => b"exact checkpoint reference".as_slice(),
        BranchStrategy::PrefixReplay => b"{}".as_slice(),
    };
    let artifact_id = artifacts.stage_blob(bytes)?.as_str().to_owned();
    let child = draft(
        "branch:selected",
        Some(ROOT),
        strategy,
        vec![BranchArtifactReference { artifact_id, role }],
    );
    let mut branch = store.create("operation:create-selected", child)?;
    let preparing = match strategy {
        BranchStrategy::ExactRestore => DurableBranchStatus::Restoring,
        BranchStrategy::PrefixReplay => DurableBranchStatus::Replaying,
    };
    branch = store.transition(
        "operation:prepare-selected",
        EXPERIMENT,
        "branch:selected",
        branch.metadata_revision,
        preparing,
    )?;
    let assurance = match strategy {
        BranchStrategy::ExactRestore => BranchAssurance::ExactRestoreReceipt,
        BranchStrategy::PrefixReplay => BranchAssurance::PrefixReplayBoundary,
    };
    branch = store.set_assurance(
        "operation:assure-selected",
        EXPERIMENT,
        "branch:selected",
        branch.metadata_revision,
        assurance,
    )?;
    store.transition(
        "operation:ready-selected",
        EXPERIMENT,
        "branch:selected",
        branch.metadata_revision,
        DurableBranchStatus::Ready,
    )?;
    Ok(())
}

fn draft(
    branch_id: &str,
    parent_branch_id: Option<&str>,
    strategy: BranchStrategy,
    artifacts: Vec<BranchArtifactReference>,
) -> DurableBranchDraft {
    let exact = strategy == BranchStrategy::ExactRestore;
    DurableBranchDraft {
        experiment_id: EXPERIMENT.to_owned(),
        root_branch_id: ROOT.to_owned(),
        branch_id: branch_id.to_owned(),
        parent_branch_id: parent_branch_id.map(str::to_owned),
        fork: BranchFork {
            occurrence_id: OccurrenceId::parse(&format!("occurrence:{branch_id}"))
                .expect("valid occurrence"),
            parent_occurrence_id: parent_branch_id
                .map(|_| OccurrenceId::parse("occurrence:branch:root").expect("valid parent")),
            state_digest: state(),
        },
        strategy,
        source_handle: exact.then(|| String::from("checkpoint-source:selected")),
        trajectory_prefix: (!exact).then(|| String::from("trajectory-prefix:selected")),
        effective_seed: Some(String::from("seed:selected")),
        setup_digest: Some(String::from("setup:selected")),
        boundary: String::from("decision"),
        assurance: BranchAssurance::Unverified,
        run_id: format!("run:{branch_id}"),
        episode_id: Some(format!("episode:{branch_id}")),
        trajectory_id: Some(format!("trajectory:{branch_id}")),
        context_id: Some(format!("context:{branch_id}")),
        policy_revision: String::from("policy:current"),
        config_revision: String::from("config:current"),
        name: branch_id.to_owned(),
        notes: None,
        artifacts,
    }
}

fn select(
    branch_store_path: &Path,
    artifact_store_path: &Path,
) -> Result<SelectedBranchContinuation, String> {
    let selector = BranchContinuationSelector::new(EXPERIMENT, "branch:selected")
        .map_err(|error| error.to_string())?;
    SelectedBranchContinuation::load(&selector, branch_store_path, artifact_store_path)
}

#[derive(Default)]
struct RecordingPort {
    calls: Vec<&'static str>,
}

impl BranchContinuationEffectPort for RecordingPort {
    type Output = &'static str;

    fn exact_restore(
        &mut self,
        _selected: &mut SelectedBranchContinuation,
    ) -> Result<Self::Output, String> {
        self.calls.push("exact_restore");
        Ok("restored")
    }

    fn prefix_replay(
        &mut self,
        selected: &mut SelectedBranchContinuation,
        prefix: &[u8],
    ) -> Result<Self::Output, String> {
        self.calls.push("prefix_replay");
        if prefix != b"{}" {
            return Err(String::from("unexpected replay artifact bytes"));
        }
        selected.claim_prefix_replay()?;
        selected.publish_prefix_boundary()?;
        Ok("replayed")
    }
}

#[test]
fn production_dispatch_round_trips_exact_strategy_through_typed_effect_port()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::ExactRestore)?;
    let mut selected = select(&branch_store, &artifact_path)?;
    let mut port = RecordingPort::default();

    assert_eq!(selected.branch().status, DurableBranchStatus::Ready);
    assert!(matches!(
        selected.strategy(),
        BranchContinuationStrategyPlan::ExactRestore { .. }
    ));
    assert_eq!(dispatch(&mut selected, &mut port)?, "restored");
    assert_eq!(port.calls, ["exact_restore"]);
    assert_eq!(selected.branch().status, DurableBranchStatus::Ready);
    Ok(())
}

#[test]
fn production_dispatch_claims_prefix_by_cas_then_publishes_verified_running_branch()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;
    let mut selected = select(&branch_store, &artifact_path)?;
    let initial_revision = selected.branch().metadata_revision;
    let mut port = RecordingPort::default();

    assert!(matches!(
        selected.strategy(),
        BranchContinuationStrategyPlan::PrefixReplay { .. }
    ));
    assert_eq!(selected.replay_prefix(), Some(b"{}".as_slice()));
    assert_eq!(dispatch(&mut selected, &mut port)?, "replayed");
    assert_eq!(port.calls, ["prefix_replay"]);

    let persisted = SqliteBranchStore::open(&branch_store)?
        .get(EXPERIMENT, "branch:selected")?
        .expect("selected branch persists");
    assert_eq!(persisted.status, DurableBranchStatus::Running);
    assert_eq!(persisted.assurance, BranchAssurance::PrefixReplayBoundary);
    assert_eq!(persisted.metadata_revision, initial_revision + 4);
    Ok(())
}

#[test]
fn running_branch_is_refused_without_changing_its_owner_state()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;
    let mut selected = select(&branch_store, &artifact_path)?;
    selected.claim_prefix_replay()?;
    selected.publish_prefix_boundary()?;
    let before = SqliteBranchStore::open(&branch_store)?
        .get(EXPERIMENT, "branch:selected")?
        .expect("running selected branch");
    assert_eq!(before.status, DurableBranchStatus::Running);

    let selector =
        BranchContinuationSelector::new(EXPERIMENT, "branch:selected").expect("selector");
    let error = SelectedBranchContinuation::load(&selector, &branch_store, &artifact_path)
        .err()
        .expect("running branch must be refused");
    assert!(error.contains("destination lease/session ownership evidence is unavailable"));

    let after = SqliteBranchStore::open(&branch_store)?
        .get(EXPERIMENT, "branch:selected")?
        .expect("selected branch remains");
    assert_eq!(after.status, DurableBranchStatus::Running);
    assert_eq!(after.metadata_revision, before.metadata_revision);
    Ok(())
}

#[test]
fn selected_child_run_identities_are_bound_before_runtime_admission() -> Result<(), String> {
    let mut config = super::super::config::RuntimeConfig {
        seed_transport: Some(
            super::super::seed_transport::SeedTransportConfig::fixture_for_tests(
                "seed:selected",
                "seed-operation:selected",
            ),
        ),
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run:parent"),
        episode_id: String::from("episode:parent"),
        trajectory_id: String::from("trajectory:parent"),
        trace_id: String::from("trace:parent"),
        artifact_id: String::from("artifact:parent"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    };
    let temp = TemporaryDirectory::new().map_err(|error| error.to_string())?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)
        .map_err(|error| error.to_string())?;
    let selected = select(&branch_store, &artifact_path)?;

    bind_branch_identities(&selected, &mut config)?;

    assert_eq!(config.run_id, "run:branch:selected");
    assert_eq!(config.episode_id, "episode:branch:selected");
    assert_eq!(config.trajectory_id, "trajectory:branch:selected");
    Ok(())
}
