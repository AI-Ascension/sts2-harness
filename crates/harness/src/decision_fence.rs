// SPDX-License-Identifier: MIT

//! In-flight decision fencing across execution epochs.
//!
//! A decision is issued a token bound to the current execution epoch. When a restore, branch
//! switch, or authority change opens a new epoch, every outstanding token becomes stale and is
//! refused at settlement even if the exact state digest is unchanged. Tokens are also settled at
//! most once, so a duplicated or replayed response cannot be applied twice.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Maximum outstanding decisions tracked at once.
pub const MAX_OUTSTANDING_DECISIONS: usize = 4096;

/// Rejection reasons for decision fencing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FenceError {
    /// The request targets an epoch that is not current.
    StaleEpoch,
    /// The new epoch does not advance the current one.
    NonAdvancingEpoch,
    /// The token belongs to an epoch that was invalidated.
    StaleDecision,
    /// The token was never issued or was already settled.
    UnknownDecision,
    /// Too many decisions are outstanding.
    Capacity,
}

impl fmt::Display for FenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StaleEpoch => "decision targets a stale execution epoch",
            Self::NonAdvancingEpoch => "new execution epoch does not advance the current one",
            Self::StaleDecision => "decision was invalidated by a later execution epoch",
            Self::UnknownDecision => "decision token is unknown or already settled",
            Self::Capacity => "too many decisions are outstanding",
        })
    }
}

impl std::error::Error for FenceError {}

/// A token issued for one decision in one execution epoch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DecisionToken {
    /// Epoch the decision was issued under.
    pub execution_epoch: u64,
    /// Monotonic sequence within the fence.
    pub sequence: u64,
}

/// Fence tracking outstanding decisions per execution epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DecisionFence {
    current_epoch: u64,
    next_sequence: u64,
    outstanding: BTreeMap<u64, BTreeSet<u64>>,
    invalidated: u64,
}

impl DecisionFence {
    /// Creates a fence at the given execution epoch.
    #[must_use]
    pub fn new(execution_epoch: u64) -> Self {
        Self {
            current_epoch: execution_epoch,
            next_sequence: 1,
            outstanding: BTreeMap::new(),
            invalidated: 0,
        }
    }

    /// Returns the current execution epoch.
    #[must_use]
    pub const fn current_epoch(&self) -> u64 {
        self.current_epoch
    }

    /// Returns the number of decisions awaiting settlement.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.outstanding.values().map(BTreeSet::len).sum()
    }

    /// Returns how many decisions have been invalidated by epoch changes.
    #[must_use]
    pub const fn invalidated(&self) -> u64 {
        self.invalidated
    }

    /// Issues a token for the current epoch.
    pub fn issue(&mut self, execution_epoch: u64) -> Result<DecisionToken, FenceError> {
        if execution_epoch != self.current_epoch {
            return Err(FenceError::StaleEpoch);
        }
        if self.outstanding() >= MAX_OUTSTANDING_DECISIONS {
            return Err(FenceError::Capacity);
        }
        let token = DecisionToken {
            execution_epoch,
            sequence: self.next_sequence,
        };
        self.next_sequence += 1;
        self.outstanding
            .entry(execution_epoch)
            .or_default()
            .insert(token.sequence);
        Ok(token)
    }

    /// Settles a token exactly once, refusing tokens from invalidated epochs.
    pub fn settle(&mut self, token: DecisionToken) -> Result<(), FenceError> {
        if token.execution_epoch != self.current_epoch {
            return Err(FenceError::StaleDecision);
        }
        let settled = self
            .outstanding
            .get_mut(&token.execution_epoch)
            .is_some_and(|sequences| sequences.remove(&token.sequence));
        if settled {
            Ok(())
        } else {
            Err(FenceError::UnknownDecision)
        }
    }

    /// Opens a new epoch, invalidating every outstanding decision and returning how many.
    pub fn advance_epoch(&mut self, execution_epoch: u64) -> Result<usize, FenceError> {
        if execution_epoch <= self.current_epoch {
            return Err(FenceError::NonAdvancingEpoch);
        }
        let stale: usize = self
            .outstanding
            .range(..execution_epoch)
            .map(|(_, sequences)| sequences.len())
            .sum();
        self.outstanding
            .retain(|epoch, _| *epoch >= execution_epoch);
        self.current_epoch = execution_epoch;
        self.invalidated += u64::try_from(stale).unwrap_or(u64::MAX);
        Ok(stale)
    }
}
