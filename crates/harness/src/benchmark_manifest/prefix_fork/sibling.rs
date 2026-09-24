// SPDX-License-Identifier: MIT

//! Sibling forks created from one shared source prefix.
//!
//! Every sibling keeps the same source prefix and receives its own trajectory, operation and
//! context identities, so two continuations of one boundary never share authority.

use std::fmt;

use super::label_ok;

/// Maximum siblings one prefix may fork.
pub const MAX_SIBLINGS: usize = 32;

/// One sibling continuation of a shared source prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SiblingFork {
    /// Stable trajectory identity, distinct from the shared source prefix.
    pub trajectory_id: String,
    /// Operation identity reserved for this sibling.
    pub operation_id: String,
    /// Provider/context namespace used by this sibling alone.
    pub context_id: String,
    /// The different legal next decision this sibling dispatches.
    pub next_action: String,
}

/// Rejection reasons for sibling creation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiblingError {
    /// A label is empty, oversized or contains a NUL separator.
    InvalidLabel,
    /// The sibling count reached [`MAX_SIBLINGS`].
    Capacity,
    /// A trajectory, operation or context identity repeats an existing sibling.
    DuplicateIdentity,
    /// The sibling was created from a different source prefix.
    ForeignPrefix,
}

impl fmt::Display for SiblingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLabel => "sibling label is invalid",
            Self::Capacity => "sibling bound is reached",
            Self::DuplicateIdentity => "sibling identity is already allocated",
            Self::ForeignPrefix => "sibling belongs to a different source prefix",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SiblingError {}

/// The siblings forked from one immutable source prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SiblingSet {
    source_prefix_digest: String,
    siblings: Vec<SiblingFork>,
}

impl SiblingSet {
    /// Creates an empty sibling set for one source prefix digest.
    ///
    /// # Errors
    ///
    /// Returns [`SiblingError::InvalidLabel`] for an invalid source prefix digest.
    pub fn new(source_prefix_digest: &str) -> Result<Self, SiblingError> {
        if !label_ok(source_prefix_digest) {
            return Err(SiblingError::InvalidLabel);
        }
        Ok(Self {
            source_prefix_digest: source_prefix_digest.to_owned(),
            siblings: Vec::new(),
        })
    }

    /// Returns the shared source prefix digest.
    #[must_use]
    pub fn source_prefix_digest(&self) -> &str {
        &self.source_prefix_digest
    }

    /// Returns the recorded siblings in creation order.
    #[must_use]
    pub fn siblings(&self) -> &[SiblingFork] {
        &self.siblings
    }

    /// Adds a sibling that keeps the shared prefix and owns fresh identities.
    ///
    /// # Errors
    ///
    /// Returns [`SiblingError::ForeignPrefix`] when the sibling names another prefix,
    /// [`SiblingError::InvalidLabel`] for an invalid label, [`SiblingError::DuplicateIdentity`]
    /// when a trajectory, operation or context identity repeats, and [`SiblingError::Capacity`]
    /// once [`MAX_SIBLINGS`] is reached.
    pub fn add(
        &mut self,
        source_prefix_digest: &str,
        fork: SiblingFork,
    ) -> Result<&SiblingFork, SiblingError> {
        if source_prefix_digest != self.source_prefix_digest {
            return Err(SiblingError::ForeignPrefix);
        }
        if self.siblings.len() >= MAX_SIBLINGS {
            return Err(SiblingError::Capacity);
        }
        for label in [
            &fork.trajectory_id,
            &fork.operation_id,
            &fork.context_id,
            &fork.next_action,
        ] {
            if !label_ok(label) {
                return Err(SiblingError::InvalidLabel);
            }
        }
        let duplicate = self.siblings.iter().any(|existing| {
            existing.trajectory_id == fork.trajectory_id
                || existing.operation_id == fork.operation_id
                || existing.context_id == fork.context_id
        });
        if duplicate {
            return Err(SiblingError::DuplicateIdentity);
        }
        self.siblings.push(fork);
        self.siblings.last().ok_or(SiblingError::Capacity)
    }
}
