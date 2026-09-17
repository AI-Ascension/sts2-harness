// SPDX-License-Identifier: MIT

//! Runtime selection, artifact loading, and CAS lifecycle for durable branch continuations.
//!
//! Strategy effects cross a typed port. Running prefix branches resume only after a durable
//! verified-boundary claim matches the live Gateway owner fence. The exact-restore route remains
//! unavailable until the fixed game-mod/MCP/Gateway restore operation is implemented.

use std::path::{Path, PathBuf};

#[path = "branch_continuation_runtime_resume_lock.rs"]
mod resume_lock;

use sts2_harness::{
    BlobDigest, BranchArtifactRole, BranchAssurance, BranchContinuationAdmission,
    BranchContinuationAdmissionError, BranchContinuationClaim, BranchContinuationClaimState,
    BranchContinuationSelector, BranchContinuationStrategyPlan, DurableBranch, DurableBranchStatus,
    ExactArtifactStore, ExactArtifactStoreResolver, SqliteBranchStore, admit_branch_continuation,
    admit_running_branch_continuation,
};

const CONTINUATION_OPERATION_PREFIX: &str = "runtime-branch-continuation";

/// The branch and all verified source bytes admitted for one runtime continuation.
pub(crate) struct SelectedBranchContinuation {
    store: SqliteBranchStore,
    admission: BranchContinuationAdmission,
    replay_prefix: Option<Vec<u8>>,
    artifact_store_path: PathBuf,
    exact_restore: Option<super::exact_restore::VerifiedClosure>,
    operation_id: String,
    metadata_revision: u64,
    owner_claim: Option<BranchContinuationClaim>,
    resuming: bool,
    _resume_lock: Option<std::fs::File>,
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
        let resume_lock = if resuming {
            Some(resume_lock::acquire(
                branch_store_path,
                selector.experiment_id(),
                selector.branch_id(),
            )?)
        } else {
            None
        };
        let store = SqliteBranchStore::open(branch_store_path)
            .map_err(|error| format!("cannot open durable branch store: {error}"))?;
        if let Some(lock) = resume_lock.as_ref() {
            resume_lock::verify(branch_store_path, lock)?;
        }
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
            if !matches!(
                claim.state,
                BranchContinuationClaimState::BoundaryVerified
                    | BranchContinuationClaimState::Resuming
            ) || claim.owner_json.is_none()
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
            artifact_store_path: artifact_store_path.to_path_buf(),
            exact_restore: None,
            operation_id,
            metadata_revision,
            owner_claim,
            resuming,
            _resume_lock: resume_lock,
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

    /// Returns the preverified exact closure admitted before owner allocation.
    pub(crate) fn exact_restore(&self) -> Option<&super::exact_restore::VerifiedClosure> {
        self.exact_restore.as_ref()
    }

    /// Installs an exact closure only after its complete source manifest and all bytes verify.
    pub(crate) fn install_exact_restore(
        &mut self,
        closure: super::exact_restore::VerifiedClosure,
    ) -> Result<(), String> {
        if !matches!(
            self.admission.strategy,
            BranchContinuationStrategyPlan::ExactRestore { .. }
        ) || self.exact_restore.is_some()
        {
            return Err(String::from(
                "verified exact-restore closure does not match the selected branch",
            ));
        }
        self.exact_restore = Some(closure);
        Ok(())
    }

    pub(crate) fn verify_persisted_exact_receipt(
        &self,
        closure: &super::exact_restore::VerifiedClosure,
    ) -> Result<(), String> {
        let receipt = self
            .admission
            .branch
            .artifacts
            .iter()
            .filter(|artifact| artifact.role == BranchArtifactRole::ContextSnapshot)
            .collect::<Vec<_>>();
        if receipt.len() != 1 {
            return Err(String::from(
                "running exact branch must retain exactly one destination receipt",
            ));
        }
        let digest = BlobDigest::parse(&receipt[0].artifact_id)
            .map_err(|_| String::from("persisted exact-restore receipt is not a verified blob"))?;
        let artifacts = ExactArtifactStore::new(&self.artifact_store_path);
        let bytes = artifacts
            .read_blob(&digest)
            .map_err(|error| format!("cannot read persisted exact-restore receipt: {error}"))?;
        super::exact_restore::operation::verify_persisted_receipt(&bytes, self, closure)
    }

    pub(crate) fn is_exact_restore(&self) -> bool {
        matches!(
            self.admission.strategy,
            BranchContinuationStrategyPlan::ExactRestore { .. }
        )
    }
}

include!("branch_continuation_runtime_lifecycle.rs");

include!("branch_continuation_runtime_exact_receipt.rs");

include!("branch_continuation_runtime_resume_claim.rs");

include!("branch_continuation_runtime_dispatch.rs");

include!("branch_continuation_runtime_binding.rs");

include!("branch_continuation_runtime_artifact_path.rs");

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

#[cfg(test)]
#[path = "branch_continuation_runtime_tests.rs"]
mod tests;
