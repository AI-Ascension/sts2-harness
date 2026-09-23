// SPDX-License-Identifier: MIT

//! Bounded rejection vocabulary for the cold-launch lifecycle.

use std::fmt;

use super::baseline::{BaselineError, BaselineMismatch};
use super::process::ProcessError;

/// Rejection reasons for a cold-launch admission, transition or allocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ColdLaunchError {
    /// The trial key is empty or oversized.
    InvalidTrialKey,
    /// The concurrent-allocation bound is outside its supported range.
    InvalidAllocationBound,
    /// The requested transition is not legal from the current stage.
    IllegalTransition {
        /// Stage the trial is in.
        from: &'static str,
        /// Stage the requested transition leads to.
        to: &'static str,
    },
    /// The baseline declaration was rejected.
    Baseline(BaselineError),
    /// The declared baseline is not interchangeable with the admitted reference.
    BaselineMismatch {
        /// Every differing category, in stable order.
        reasons: Vec<BaselineMismatch>,
    },
    /// A process birth or readiness proof was rejected.
    Process(ProcessError),
    /// The destination is already leased by a different trial.
    DestinationLeased {
        /// Destination that is already held.
        destination_id: String,
        /// Trial that currently holds it.
        trial_key: String,
    },
    /// The concurrent-allocation bound has been reached.
    AllocationBoundReached,
    /// The destination was quarantined and cannot be reused.
    DestinationQuarantined(String),
    /// A reconciled reply belongs to a different trial identity.
    ForeignReconciliation {
        /// Trial whose identity did not match.
        trial_key: String,
    },
    /// A birth token is already live for a different trial.
    ReusedLiveProcess {
        /// Birth token that was already attested.
        birth_token: String,
    },
}

impl fmt::Display for ColdLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidTrialKey => "cold-launch trial key is invalid",
            Self::InvalidAllocationBound => "cold-launch allocation bound is invalid",
            Self::IllegalTransition { .. } => "cold-launch stage transition is not legal",
            Self::Baseline(_) => "pristine baseline is invalid",
            Self::BaselineMismatch { .. } => "baseline differs from the admitted reference",
            Self::Process(_) => "process attestation is invalid",
            Self::DestinationLeased { .. } => "destination is already leased",
            Self::AllocationBoundReached => "cold-launch allocation bound is reached",
            Self::DestinationQuarantined(_) => "destination is quarantined",
            Self::ForeignReconciliation { .. } => "reply belongs to another trial",
            Self::ReusedLiveProcess { .. } => "birth token is already live",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ColdLaunchError {}

impl From<BaselineError> for ColdLaunchError {
    fn from(error: BaselineError) -> Self {
        Self::Baseline(error)
    }
}

impl From<ProcessError> for ColdLaunchError {
    fn from(error: ProcessError) -> Self {
        Self::Process(error)
    }
}
