// SPDX-License-Identifier: MIT

//! The versioned, capability-gated readiness target.
//!
//! An authored workflow names the milestone it waits for and a bounded deadline.
//! The target is versioned so a consumer that does not implement this contract
//! rejects an unsupported target deterministically before any work starts,
//! instead of silently ignoring fields it does not understand. Unknown members
//! are refused at deserialization time by `deny_unknown_fields`; an unknown
//! version value is refused by [`ReadinessTarget::new`].

use serde::{Deserialize, Serialize};

use super::{ReadinessMilestone, ReadinessWaitError};

/// The readiness-target contract version this harness implements.
pub const READINESS_CONTRACT_VERSION: u32 = 1;

/// Default bounded deadline for [`ReadinessTarget::standard`], in milliseconds.
pub const STANDARD_READINESS_DEADLINE_MS: u64 = 30_000;

/// Default bounded attempt budget for [`ReadinessTarget::standard`].
pub const STANDARD_READINESS_MAX_ATTEMPTS: u32 = 60;

/// A versioned milestone target with a bounded deadline and attempt budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessTarget {
    /// The milestone the wait must reach.
    pub milestone: ReadinessMilestone,
    /// The wall-clock budget, in milliseconds, measured from the wait's start.
    pub deadline_ms: u64,
    /// The maximum number of admitted observations before the wait times out.
    pub max_attempts: u32,
    /// The contract version this target was authored against.
    pub contract_version: u32,
}

impl ReadinessTarget {
    /// Admits a target, refusing an unsupported version before any work starts.
    ///
    /// Fails closed with [`ReadinessWaitError::Incompatible`] when the version is
    /// not [`READINESS_CONTRACT_VERSION`], and with
    /// [`ReadinessWaitError::InvalidTarget`] when the deadline or attempt budget
    /// is zero (an unbounded wait is not a target).
    pub fn new(
        milestone: ReadinessMilestone,
        deadline_ms: u64,
        max_attempts: u32,
        contract_version: u32,
    ) -> Result<Self, ReadinessWaitError> {
        if contract_version != READINESS_CONTRACT_VERSION {
            return Err(ReadinessWaitError::Incompatible);
        }
        if deadline_ms == 0 || max_attempts == 0 {
            return Err(ReadinessWaitError::InvalidTarget);
        }
        Ok(Self {
            milestone,
            deadline_ms,
            max_attempts,
            contract_version,
        })
    }

    /// A target at the current contract version with the standard bounds.
    #[must_use]
    pub const fn standard(milestone: ReadinessMilestone) -> Self {
        Self {
            milestone,
            deadline_ms: STANDARD_READINESS_DEADLINE_MS,
            max_attempts: STANDARD_READINESS_MAX_ATTEMPTS,
            contract_version: READINESS_CONTRACT_VERSION,
        }
    }

    /// Re-validates a target that was deserialized from an untrusted document.
    pub fn validate(self) -> Result<Self, ReadinessWaitError> {
        Self::new(
            self.milestone,
            self.deadline_ms,
            self.max_attempts,
            self.contract_version,
        )
    }
}
