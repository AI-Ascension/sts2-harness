// SPDX-License-Identifier: MIT

//! The selected settled boundary and the observed replay of the prefix that reaches it.

use std::fmt;

use serde::Serialize;

use super::label_ok;

/// Maximum decision ordinal a fork boundary may select.
pub const MAX_FORK_ORDINAL: u32 = 1_000_000;
/// Maximum settled-action receipts one prefix boundary may carry.
pub const MAX_PREFIX_RECEIPTS: usize = 4096;

/// Resolution of the legal-action rebinding at the boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LegalBinding {
    /// Exactly one legal action is bound, identified by its stable action key.
    Resolved {
        /// Stable semantic action key bound at the boundary.
        action_key: String,
    },
    /// The recorded actions do not resolve to one legal next action.
    Ambiguous,
}

/// One selected settled boundary within a recorded seeded replay prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixBoundary {
    /// Stable identifier of the settled occurrence selected as the fork point.
    pub occurrence: String,
    /// Decision ordinal of the boundary within the recorded prefix.
    pub ordinal: u32,
    /// Digest of the exact state captured at the boundary.
    pub state_digest: String,
    /// Whether the selected occurrence is a settled decision boundary.
    pub settled: bool,
    /// Whether the recorded source run terminated at or before this occurrence.
    pub terminal: bool,
    /// Receipt identifiers for the settled actions replayed to reach the boundary.
    pub receipts: Vec<String>,
    /// Number of receipts the settled prefix requires.
    pub expected_receipts: u32,
    /// Resolution of the legal-action rebinding at the boundary.
    pub legal_binding: LegalBinding,
}

/// Rejection reasons for a boundary declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrefixBoundaryError {
    /// A label is empty, oversized or contains a NUL separator.
    InvalidLabel,
    /// The ordinal exceeds [`MAX_FORK_ORDINAL`].
    OrdinalOutOfRange,
    /// More than [`MAX_PREFIX_RECEIPTS`] receipts were declared.
    TooManyReceipts,
    /// A resolved legal binding carries no action key.
    InvalidBinding,
}

impl fmt::Display for PrefixBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLabel => "prefix boundary label is invalid",
            Self::OrdinalOutOfRange => "prefix boundary ordinal is out of range",
            Self::TooManyReceipts => "prefix boundary declares too many receipts",
            Self::InvalidBinding => "prefix boundary legal binding is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PrefixBoundaryError {}

/// Result of replaying the prefix up to the selected boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReplayObservation {
    /// The prefix replayed exactly to the boundary.
    Verified {
        /// Number of settled actions replayed.
        settled_actions: u32,
        /// Provider calls made while replaying the prefix; a prefix replay makes none.
        provider_calls: u32,
    },
    /// The replay diverged from the recorded prefix at this ordinal.
    Diverged {
        /// Ordinal at which the replay first diverged.
        ordinal: u32,
    },
    /// The replay stopped before reaching the boundary.
    Incomplete {
        /// Settled actions actually replayed.
        replayed: u32,
        /// Settled actions the boundary requires.
        expected: u32,
    },
}

impl PrefixBoundary {
    /// Validates labels, the ordinal bound, the receipt bound and the binding.
    ///
    /// # Errors
    ///
    /// Returns the specific [`PrefixBoundaryError`] for the first failing field.
    pub fn validate(&self) -> Result<(), PrefixBoundaryError> {
        for value in [self.occurrence.as_str(), self.state_digest.as_str()] {
            if !label_ok(value) {
                return Err(PrefixBoundaryError::InvalidLabel);
            }
        }
        for receipt in &self.receipts {
            if !label_ok(receipt) {
                return Err(PrefixBoundaryError::InvalidLabel);
            }
        }
        if self.ordinal > MAX_FORK_ORDINAL {
            return Err(PrefixBoundaryError::OrdinalOutOfRange);
        }
        if self.receipts.len() > MAX_PREFIX_RECEIPTS {
            return Err(PrefixBoundaryError::TooManyReceipts);
        }
        if let LegalBinding::Resolved { action_key } = &self.legal_binding
            && !label_ok(action_key)
        {
            return Err(PrefixBoundaryError::InvalidBinding);
        }
        Ok(())
    }
}
