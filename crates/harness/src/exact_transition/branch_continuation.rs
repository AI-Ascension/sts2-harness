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

include!("branch_continuation_admission.rs");
