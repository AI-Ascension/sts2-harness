// SPDX-License-Identifier: MIT

//! Read-only admission for an explicitly selected durable branch continuation.
//!
//! This module validates persisted readiness, strategy evidence, descriptor shape, and retained
//! artifact availability. It does not perform a restore or replay, acquire a destination, or prove
//! that a prior receipt applies to a new destination. A runtime consumer must execute the returned
//! strategy plan through its owning port and verify fresh destination evidence before decisions.
//! Running prefix branches use a separate resume admission that requires a verified boundary and a
//! fresh current-owner fence.

use std::fmt;

use super::{
    BranchArtifactAvailability, BranchArtifactReference, BranchArtifactResolver,
    BranchArtifactRole, BranchAssurance, BranchStoreError, BranchStrategy, DurableBranch,
    DurableBranchStatus, MAX_TRANSITION_LABEL_BYTES, SqliteBranchStore,
};

/// Stable, explicit branch selector. Experiment and branch identity remain separate namespaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchContinuationSelector {
    experiment_id: String,
    branch_id: String,
}

impl BranchContinuationSelector {
    /// Validates and creates an explicit experiment/branch selector.
    ///
    /// # Errors
    ///
    /// Returns [`BranchContinuationAdmissionError::InvalidSelector`] for an empty, oversized, or
    /// NUL-containing identity.
    pub fn new(
        experiment_id: impl Into<String>,
        branch_id: impl Into<String>,
    ) -> Result<Self, BranchContinuationAdmissionError> {
        let experiment_id = experiment_id.into();
        let branch_id = branch_id.into();
        if !valid_selector_part(&experiment_id) || !valid_selector_part(&branch_id) {
            return Err(BranchContinuationAdmissionError::InvalidSelector);
        }
        Ok(Self {
            experiment_id,
            branch_id,
        })
    }

    /// Returns the selected experiment identity.
    #[must_use]
    pub fn experiment_id(&self) -> &str {
        &self.experiment_id
    }

    /// Returns the selected immutable branch identity.
    #[must_use]
    pub fn branch_id(&self) -> &str {
        &self.branch_id
    }
}

/// Strategy-specific source material required by the runtime dispatcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BranchContinuationStrategyPlan {
    /// Restore from the retained exact checkpoint through the checkpoint owner.
    ExactRestore {
        checkpoint: BranchArtifactReference,
        restore_closure: Vec<BranchArtifactReference>,
    },
    /// Replay the retained public prefix through the replay owner.
    PrefixReplay {
        replay_prefix: BranchArtifactReference,
    },
}

/// Read-only plan produced after a selected ready branch passes admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchContinuationAdmission {
    /// Full persisted branch descriptor, including independent run and episode identities.
    pub branch: DurableBranch,
    /// Validated strategy-specific artifact references for dispatch.
    pub strategy: BranchContinuationStrategyPlan,
    /// Resolution of every retained reference, including context and restore-closure artifacts.
    pub artifacts: BranchArtifactAvailability,
}

/// Refusal reasons for explicit branch continuation admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BranchContinuationAdmissionError {
    /// An experiment or branch identity is empty, oversized, or contains NUL.
    InvalidSelector,
    /// The selected branch does not exist in the requested experiment.
    Store(BranchStoreError),
    /// The selected branch is not ready for continuation.
    BranchNotReady {
        /// Persisted status at the time of admission.
        status: DurableBranchStatus,
    },
    /// The stored strategy evidence does not match the selected strategy.
    AssuranceMismatch {
        /// Strategy recorded in the branch.
        strategy: BranchStrategy,
        /// Evidence recorded in the branch.
        assurance: BranchAssurance,
    },
    /// The branch's mutually exclusive strategy source fields are inconsistent.
    InvalidStrategyDescriptor {
        /// Strategy recorded in the branch.
        strategy: BranchStrategy,
    },
    /// The branch does not retain the one required source artifact for its strategy.
    MissingStrategyArtifact {
        /// Required artifact role.
        role: BranchArtifactRole,
    },
    /// More than one artifact has the strategy's required source role.
    AmbiguousStrategyArtifact {
        /// Required artifact role.
        role: BranchArtifactRole,
    },
    /// One or more retained artifacts are missing or could not be verified.
    ArtifactsUnavailable(BranchArtifactAvailability),
}

impl fmt::Display for BranchContinuationAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelector => formatter.write_str("branch continuation selector is invalid"),
            Self::Store(error) => write!(formatter, "branch continuation lookup failed: {error}"),
            Self::BranchNotReady { status } => {
                write!(
                    formatter,
                    "branch is not ready for continuation: {status:?}"
                )
            }
            Self::AssuranceMismatch {
                strategy,
                assurance,
            } => write!(
                formatter,
                "branch strategy {strategy:?} does not admit evidence {assurance:?}"
            ),
            Self::InvalidStrategyDescriptor { strategy } => {
                write!(
                    formatter,
                    "branch {strategy:?} source descriptor is inconsistent"
                )
            }
            Self::MissingStrategyArtifact { role } => {
                write!(formatter, "branch has no required {role:?} artifact")
            }
            Self::AmbiguousStrategyArtifact { role } => {
                write!(formatter, "branch has multiple required {role:?} artifacts")
            }
            Self::ArtifactsUnavailable(_) => {
                formatter.write_str("one or more retained branch artifacts are unavailable")
            }
        }
    }
}

impl std::error::Error for BranchContinuationAdmissionError {}

/// Loads and validates one explicitly selected ready branch without performing external effects.
///
/// The returned metadata revision is the CAS token a runtime consumer must use when it claims the
/// continuation. Strategy assurance in the stored row confirms prior preparation only; the runtime
/// must obtain and verify evidence for the destination it is about to use.
///
/// # Errors
///
/// Returns a typed error for an unknown branch, a non-ready status, mismatched assurance, malformed
/// strategy source fields, absent/ambiguous source artifacts, or unavailable retained artifacts.
pub fn admit_branch_continuation(
    store: &SqliteBranchStore,
    selector: &BranchContinuationSelector,
    resolver: &dyn BranchArtifactResolver,
) -> Result<BranchContinuationAdmission, BranchContinuationAdmissionError> {
    let branch = store
        .get(selector.experiment_id(), selector.branch_id())
        .map_err(BranchContinuationAdmissionError::Store)?
        .ok_or(BranchContinuationAdmissionError::Store(
            BranchStoreError::UnknownBranch,
        ))?;
    if branch.status != DurableBranchStatus::Ready {
        return Err(BranchContinuationAdmissionError::BranchNotReady {
            status: branch.status,
        });
    }

    let strategy = strategy_plan(&branch)?;
    let artifacts = store
        .artifact_availability(selector.experiment_id(), selector.branch_id(), resolver)
        .map_err(BranchContinuationAdmissionError::Store)?;
    if !artifacts.all_available() {
        return Err(BranchContinuationAdmissionError::ArtifactsUnavailable(
            artifacts,
        ));
    }
    Ok(BranchContinuationAdmission {
        branch,
        strategy,
        artifacts,
    })
}

/// Admits a selected running prefix branch for explicit execution-store resume.
///
/// This path requires the prefix boundary assurance and every retained artifact to remain
/// available. It does not replay the prefix. A runtime caller must additionally prove the same
/// live gateway owner and historical continuation claim before opening the decision path.
///
/// # Errors
///
/// Returns a typed error when the branch is not running, is not a prefix-replay branch, lacks its
/// verified prefix boundary, or has unavailable retained artifacts.
pub fn admit_running_branch_continuation(
    store: &SqliteBranchStore,
    selector: &BranchContinuationSelector,
    resolver: &dyn BranchArtifactResolver,
) -> Result<BranchContinuationAdmission, BranchContinuationAdmissionError> {
    let branch = store
        .get(selector.experiment_id(), selector.branch_id())
        .map_err(BranchContinuationAdmissionError::Store)?
        .ok_or(BranchContinuationAdmissionError::Store(
            BranchStoreError::UnknownBranch,
        ))?;
    if branch.status != DurableBranchStatus::Running {
        return Err(BranchContinuationAdmissionError::BranchNotReady {
            status: branch.status,
        });
    }
    if branch.strategy != BranchStrategy::PrefixReplay {
        return Err(
            BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                strategy: branch.strategy,
            },
        );
    }
    let strategy = strategy_plan(&branch)?;
    if !matches!(
        strategy,
        BranchContinuationStrategyPlan::PrefixReplay { .. }
    ) {
        return Err(
            BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                strategy: branch.strategy,
            },
        );
    }
    let artifacts = store
        .artifact_availability(selector.experiment_id(), selector.branch_id(), resolver)
        .map_err(BranchContinuationAdmissionError::Store)?;
    if !artifacts.all_available() {
        return Err(BranchContinuationAdmissionError::ArtifactsUnavailable(
            artifacts,
        ));
    }
    Ok(BranchContinuationAdmission {
        branch,
        strategy,
        artifacts,
    })
}

fn strategy_plan(
    branch: &DurableBranch,
) -> Result<BranchContinuationStrategyPlan, BranchContinuationAdmissionError> {
    let expected_assurance = match branch.strategy {
        BranchStrategy::ExactRestore => BranchAssurance::ExactRestoreReceipt,
        BranchStrategy::PrefixReplay => BranchAssurance::PrefixReplayBoundary,
    };
    if branch.assurance != expected_assurance {
        return Err(BranchContinuationAdmissionError::AssuranceMismatch {
            strategy: branch.strategy,
            assurance: branch.assurance,
        });
    }

    let (required_role, plan) = match branch.strategy {
        BranchStrategy::ExactRestore => {
            if branch.source_handle.is_none() || branch.trajectory_prefix.is_some() {
                return Err(
                    BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                        strategy: branch.strategy,
                    },
                );
            }
            let checkpoint = unique_artifact(&branch.artifacts, BranchArtifactRole::Checkpoint)?;
            let restore_closure = branch
                .artifacts
                .iter()
                .filter(|artifact| artifact.role == BranchArtifactRole::RestoreClosure)
                .cloned()
                .collect();
            (
                BranchArtifactRole::Checkpoint,
                BranchContinuationStrategyPlan::ExactRestore {
                    checkpoint,
                    restore_closure,
                },
            )
        }
        BranchStrategy::PrefixReplay => {
            if branch.trajectory_prefix.is_none() || branch.source_handle.is_some() {
                return Err(
                    BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                        strategy: branch.strategy,
                    },
                );
            }
            let replay_prefix =
                unique_artifact(&branch.artifacts, BranchArtifactRole::ReplayPrefix)?;
            (
                BranchArtifactRole::ReplayPrefix,
                BranchContinuationStrategyPlan::PrefixReplay { replay_prefix },
            )
        }
    };

    if branch
        .artifacts
        .iter()
        .any(|artifact| artifact.role == opposite_strategy_role(required_role))
    {
        return Err(
            BranchContinuationAdmissionError::InvalidStrategyDescriptor {
                strategy: branch.strategy,
            },
        );
    }
    Ok(plan)
}

fn unique_artifact(
    artifacts: &[BranchArtifactReference],
    role: BranchArtifactRole,
) -> Result<BranchArtifactReference, BranchContinuationAdmissionError> {
    let mut matching = artifacts.iter().filter(|artifact| artifact.role == role);
    let Some(first) = matching.next() else {
        return Err(BranchContinuationAdmissionError::MissingStrategyArtifact { role });
    };
    if matching.next().is_some() {
        return Err(BranchContinuationAdmissionError::AmbiguousStrategyArtifact { role });
    }
    Ok(first.clone())
}

const fn opposite_strategy_role(role: BranchArtifactRole) -> BranchArtifactRole {
    match role {
        BranchArtifactRole::Checkpoint => BranchArtifactRole::ReplayPrefix,
        BranchArtifactRole::ReplayPrefix => BranchArtifactRole::Checkpoint,
        BranchArtifactRole::RestoreClosure | BranchArtifactRole::ContextSnapshot => role,
    }
}

fn valid_selector_part(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TRANSITION_LABEL_BYTES && !value.contains('\0')
}
