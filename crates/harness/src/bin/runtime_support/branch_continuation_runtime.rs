// SPDX-License-Identifier: MIT

//! Runtime selection, artifact loading, and CAS lifecycle for durable branch continuations.
//!
//! Strategy effects cross a typed port. Running prefix branches resume only after a durable
//! verified-boundary claim matches the live Gateway owner fence. The exact-restore route remains
//! unavailable until the fixed game-mod/MCP/Gateway restore operation is implemented.

use std::path::{Path, PathBuf};

use sts2_harness::{
    BlobDigest, BranchAssurance, BranchContinuationAdmission, BranchContinuationAdmissionError,
    BranchContinuationClaim, BranchContinuationClaimState, BranchContinuationSelector,
    BranchContinuationStrategyPlan, DurableBranch, DurableBranchStatus, ExactArtifactStore,
    ExactArtifactStoreResolver, SqliteBranchStore, admit_branch_continuation,
    admit_running_branch_continuation,
};

const CONTINUATION_OPERATION_PREFIX: &str = "runtime-branch-continuation";

/// The branch and all verified source bytes admitted for one runtime continuation.
pub(crate) struct SelectedBranchContinuation {
    store: SqliteBranchStore,
    admission: BranchContinuationAdmission,
    replay_prefix: Option<Vec<u8>>,
    operation_id: String,
    metadata_revision: u64,
    owner_claim: Option<BranchContinuationClaim>,
    resuming: bool,
}

impl SelectedBranchContinuation {
    /// Loads one explicitly selected Ready branch and resolves every retained artifact.
    pub(crate) fn load(
        selector: &BranchContinuationSelector,
        branch_store_path: &Path,
        artifact_store_path: &Path,
    ) -> Result<Self, String> {
        Self::load_selected(selector, branch_store_path, artifact_store_path, false)
    }

    /// Loads a running prefix branch only when its persisted claim proves a verified boundary.
    pub(crate) fn load_for_resume(
        selector: &BranchContinuationSelector,
        branch_store_path: &Path,
        artifact_store_path: &Path,
    ) -> Result<Self, String> {
        Self::load_selected(selector, branch_store_path, artifact_store_path, true)
    }

    fn load_selected(
        selector: &BranchContinuationSelector,
        branch_store_path: &Path,
        artifact_store_path: &Path,
        resuming: bool,
    ) -> Result<Self, String> {
        let store = SqliteBranchStore::open(branch_store_path)
            .map_err(|error| format!("cannot open durable branch store: {error}"))?;
        let artifacts = ExactArtifactStore::new(artifact_store_path);
        let resolver = ExactArtifactStoreResolver::new(&artifacts);
        let admission = if resuming {
            admit_running_branch_continuation(&store, selector, &resolver)
        } else {
            admit_branch_continuation(&store, selector, &resolver)
        }
            .map_err(|error| match error {
                BranchContinuationAdmissionError::BranchNotReady {
                    status: DurableBranchStatus::Running,
                } => String::from(
                    "selected branch is running; use explicit resume with verified-boundary and current-owner evidence",
                ),
                error => format!("durable branch continuation admission failed: {error}"),
            })?;
        let owner_claim = if resuming {
            let claim = store
                .continuation_claim(selector.experiment_id(), selector.branch_id())
                .map_err(|error| format!("cannot read selected branch owner-claim evidence: {error}"))?
                .ok_or_else(|| {
                    String::from(
                        "running branch has no durable current-owner claim; explicit resume is refused",
                    )
                })?;
            if claim.state != BranchContinuationClaimState::BoundaryVerified
                || claim.owner_json.is_none()
                || claim.owner_digest.is_none()
            {
                return Err(String::from(
                    "running branch lacks a verified prefix boundary and owner claim; explicit resume is refused",
                ));
            }
            Some(claim)
        } else {
            None
        };
        let replay_prefix = match &admission.strategy {
            BranchContinuationStrategyPlan::ExactRestore { .. } => None,
            BranchContinuationStrategyPlan::PrefixReplay { replay_prefix } => {
                let digest = BlobDigest::parse(&replay_prefix.artifact_id)
                    .map_err(|_| String::from("branch replay prefix is not a verified blob"))?;
                Some(
                    artifacts
                        .read_blob(&digest)
                        .map_err(|error| format!("cannot read branch replay prefix: {error}"))?,
                )
            }
        };
        let operation_id = operation_id(selector, admission.branch.metadata_revision);
        let metadata_revision = admission.branch.metadata_revision;
        Ok(Self {
            store,
            admission,
            replay_prefix,
            operation_id,
            metadata_revision,
            owner_claim,
            resuming,
        })
    }

    /// Returns the persisted branch descriptor.
    pub(crate) fn branch(&self) -> &DurableBranch {
        &self.admission.branch
    }

    /// Returns the validated strategy descriptor.
    pub(crate) fn strategy(&self) -> &BranchContinuationStrategyPlan {
        &self.admission.strategy
    }

    /// Returns the content-verified replay prefix, if this branch uses prefix replay.
    pub(crate) fn replay_prefix(&self) -> Option<&[u8]> {
        self.replay_prefix.as_deref()
    }

    /// Persists and returns the stable gateway owner-claim identity before runtime effects.
    pub(crate) fn prepare_owner_claim(&mut self) -> Result<BranchContinuationClaim, String> {
        let claim = match self.owner_claim.clone() {
            Some(claim) => claim,
            None => self
                .store
                .prepare_continuation_claim(
                    &self.admission.branch.experiment_id,
                    &self.admission.branch.branch_id,
                )
                .map_err(|error| {
                    format!("cannot persist selected branch owner-claim intent: {error}")
                })?,
        };
        self.owner_claim = Some(claim.clone());
        Ok(claim)
    }

    /// Returns true when this selection is an explicit resume of a verified running branch.
    pub(crate) const fn is_resuming(&self) -> bool {
        self.resuming
    }

    /// Claims the replay attempt before the first runtime effect using the admitted CAS revision.
    pub(crate) fn claim_prefix_replay(&mut self) -> Result<(), String> {
        if !matches!(
            self.admission.strategy,
            BranchContinuationStrategyPlan::PrefixReplay { .. }
        ) {
            return Err(String::from(
                "prefix replay claim does not match the selected branch strategy",
            ));
        }
        let claimed = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "claim"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Replaying,
            )
            .map_err(|error| format!("cannot claim prefix replay branch: {error}"))?;
        self.metadata_revision = claimed.metadata_revision;
        Ok(())
    }

    /// Persists verified prefix evidence and publishes the branch as the active continuation.
    ///
    /// This runs at the verified boundary before the first live provider decision.
    pub(crate) fn publish_prefix_boundary(&mut self) -> Result<(), String> {
        if self.current_status()? != DurableBranchStatus::Replaying {
            return Err(String::from(
                "prefix replay boundary arrived outside the claimed replay state",
            ));
        }
        let assured = self
            .store
            .set_assurance(
                &operation_suffix(&self.operation_id, "assurance"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                BranchAssurance::PrefixReplayBoundary,
            )
            .map_err(|error| format!("cannot persist prefix replay evidence: {error}"))?;
        self.metadata_revision = assured.metadata_revision;
        let ready = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "ready"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Ready,
            )
            .map_err(|error| format!("cannot publish verified branch boundary: {error}"))?;
        self.metadata_revision = ready.metadata_revision;
        let running = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "running"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Running,
            )
            .map_err(|error| format!("cannot admit branch continuation: {error}"))?;
        self.metadata_revision = running.metadata_revision;
        if let Some(claim) = self.owner_claim.as_ref()
            && claim.state == BranchContinuationClaimState::Claimed
        {
            self.store
                .transition_continuation_claim(
                    &claim.operation_id,
                    BranchContinuationClaimState::Claimed,
                    BranchContinuationClaimState::BoundaryVerified,
                )
                .map_err(|error| format!("cannot persist verified owner boundary: {error}"))?;
        }
        Ok(())
    }

    /// Marks a pre-boundary failure terminal, or post-boundary uncertainty for reconciliation.
    pub(crate) fn mark_failed(&mut self, reason: &str) -> Result<(), String> {
        let status = self.current_status()?;
        let target = match status {
            DurableBranchStatus::Replaying => DurableBranchStatus::Failed,
            DurableBranchStatus::Running => DurableBranchStatus::Unknown,
            _ => return Ok(()),
        };
        let updated = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "failed"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                target,
            )
            .map_err(|error| format!("cannot record branch continuation {reason}: {error}"))?;
        self.metadata_revision = updated.metadata_revision;
        Ok(())
    }

    /// Marks a completed continuation only after the normal runner reaches a terminal state.
    pub(crate) fn complete(&mut self) -> Result<(), String> {
        let completed = self
            .store
            .transition(
                &operation_suffix(&self.operation_id, "complete"),
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
                self.metadata_revision,
                DurableBranchStatus::Completed,
            )
            .map_err(|error| format!("cannot complete durable branch continuation: {error}"))?;
        self.metadata_revision = completed.metadata_revision;
        Ok(())
    }

    fn current_status(&self) -> Result<DurableBranchStatus, String> {
        self.store
            .get(
                &self.admission.branch.experiment_id,
                &self.admission.branch.branch_id,
            )
            .map_err(|error| format!("cannot read branch continuation state: {error}"))?
            .map(|branch| branch.status)
            .ok_or_else(|| String::from("selected branch disappeared from its store"))
    }
}

/// Typed owner boundary for the two durable branch continuation strategies.
pub(crate) trait BranchContinuationEffectPort {
    /// Effect result after destination restore evidence has been verified.
    type Output;

    /// Restores the exact checkpoint into a fresh destination and verifies destination evidence.
    fn exact_restore(
        &mut self,
        selected: &mut SelectedBranchContinuation,
    ) -> Result<Self::Output, String>;

    /// Replays the retained prefix and continues the same destination after its boundary verifies.
    fn prefix_replay(
        &mut self,
        selected: &mut SelectedBranchContinuation,
        prefix: &[u8],
    ) -> Result<Self::Output, String>;
}

/// Dispatches the persisted strategy through the typed owner port.
pub(crate) fn dispatch<P: BranchContinuationEffectPort>(
    selected: &mut SelectedBranchContinuation,
    port: &mut P,
) -> Result<P::Output, String> {
    match selected.strategy() {
        BranchContinuationStrategyPlan::ExactRestore { .. } => port.exact_restore(selected),
        BranchContinuationStrategyPlan::PrefixReplay { .. } => {
            let prefix = selected
                .replay_prefix()
                .ok_or_else(|| String::from("verified replay prefix bytes are unavailable"))?
                .to_vec();
            port.prefix_replay(selected, &prefix)
        }
    }
}

/// Applies the durable branch's independently allocated run identities to the runtime config.
pub(crate) fn bind_branch_identities(
    selected: &SelectedBranchContinuation,
    config: &mut super::RuntimeConfig,
) -> Result<(), String> {
    let branch = selected.branch();
    let episode_id = branch
        .episode_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no episode identity"))?;
    let trajectory_id = branch
        .trajectory_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no trajectory identity"))?;
    let context_id = branch
        .context_id
        .as_deref()
        .ok_or_else(|| String::from("selected branch has no context identity"))?;
    for (name, value) in [
        ("branch run_id", branch.run_id.as_str()),
        ("branch episode_id", episode_id),
        ("branch trajectory_id", trajectory_id),
        ("branch context_id", context_id),
    ] {
        if !runtime_safe_identity(value) {
            return Err(format!("selected {name} is invalid"));
        }
    }
    if matches!(
        selected.strategy(),
        BranchContinuationStrategyPlan::PrefixReplay { .. }
    ) {
        let branch_seed = branch
            .effective_seed
            .as_deref()
            .ok_or_else(|| String::from("selected replay branch has no effective seed"))?;
        let runtime_seed = config
            .seed_transport
            .as_ref()
            .map(super::seed_transport::SeedTransportConfig::requested_seed)
            .ok_or_else(|| String::from("prefix continuation requires an explicit seed plan"))?;
        if branch_seed != runtime_seed {
            return Err(String::from(
                "runtime seed does not match the selected branch effective seed",
            ));
        }
    }
    config.run_id.clone_from(&branch.run_id);
    config.episode_id = episode_id.to_owned();
    config.trajectory_id = trajectory_id.to_owned();
    let branch_scope = format!("{}\0{}", branch.experiment_id, branch.branch_id);
    let scope_digest = sts2_harness::sha256_hex(branch_scope.as_bytes());
    config.trace_id = format!("branch-trace:{scope_digest}");
    config.artifact_id = format!("branch-artifact:{scope_digest}");
    config.validate()
}

/// Resolves the content-addressed artifact directory used by durable branches.
pub(crate) fn artifact_store_path() -> Result<PathBuf, String> {
    match std::env::var("STS2_EXACT_ARTIFACT_STORE_PATH") {
        Ok(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        Ok(_) => Err(String::from(
            "STS2_EXACT_ARTIFACT_STORE_PATH must not be empty",
        )),
        Err(std::env::VarError::NotPresent) => {
            let execution = std::env::var("STS2_EXECUTION_STORE_PATH")
                .unwrap_or_else(|_| String::from("harness-execution.sqlite3"));
            Ok(PathBuf::from(execution).with_file_name("harness-exact-artifacts"))
        }
        Err(std::env::VarError::NotUnicode(_)) => Err(String::from(
            "STS2_EXACT_ARTIFACT_STORE_PATH is not valid UTF-8",
        )),
    }
}

fn operation_id(selector: &BranchContinuationSelector, revision: u64) -> String {
    let identity = format!(
        "{}\0{}\0{revision}",
        selector.experiment_id(),
        selector.branch_id()
    );
    format!(
        "{CONTINUATION_OPERATION_PREFIX}:{}",
        sts2_harness::sha256_hex(identity.as_bytes())
    )
}

fn operation_suffix(base: &str, suffix: &str) -> String {
    format!("{base}:{suffix}")
}

fn runtime_safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

#[cfg(test)]
#[path = "branch_continuation_runtime_tests.rs"]
mod tests;
