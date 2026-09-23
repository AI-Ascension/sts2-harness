// SPDX-License-Identifier: MIT

//! Bounded rejection vocabulary for prefix fork admission.

use std::fmt;

use super::binding::ForkBindingMismatch;
use super::boundary::PrefixBoundaryError;

/// Refusal reasons for a prefix fork admission; a refusal authorizes no effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrefixForkRefusal {
    /// An experiment, continuation, occurrence or action label is empty or oversized.
    InvalidLabel,
    /// The boundary declaration itself was rejected.
    Boundary(PrefixBoundaryError),
    /// The requested binding is not the binding recorded with the source prefix.
    BindingMismatch {
        /// Every differing category, in stable order.
        reasons: Vec<ForkBindingMismatch>,
    },
    /// The recorded source run terminated at or before the selected boundary.
    TerminalSource,
    /// The selected boundary is not a settled decision point.
    UnresolvedAction,
    /// The prefix does not carry a receipt for every settled action.
    MissingReceipt {
        /// Receipts the settled prefix requires.
        expected: u32,
        /// Receipts the boundary actually carries.
        present: usize,
    },
    /// The recorded actions do not resolve to one legal next action.
    AmbiguousLegalBinding,
    /// The replay did not reach the boundary.
    PrefixIncomplete {
        /// Settled actions actually replayed.
        replayed: u32,
        /// Settled actions the boundary requires.
        expected: u32,
    },
    /// The replay diverged from the recorded prefix.
    ObservedDivergence {
        /// Ordinal at which the replay first diverged.
        ordinal: u32,
    },
    /// The prefix replay invoked a provider; a prefix replay makes zero provider calls.
    ProviderCallsDuringReplay {
        /// Provider calls observed during the prefix replay.
        calls: u32,
    },
}

impl fmt::Display for PrefixForkRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLabel => "prefix fork label is invalid",
            Self::Boundary(_) => "prefix boundary is invalid",
            Self::BindingMismatch { .. } => "prefix fork binding does not match the source prefix",
            Self::TerminalSource => "recorded source run already terminated",
            Self::UnresolvedAction => "selected boundary is not a settled decision point",
            Self::MissingReceipt { .. } => "prefix is missing a settled-action receipt",
            Self::AmbiguousLegalBinding => "boundary legal-action binding is ambiguous",
            Self::PrefixIncomplete { .. } => "prefix replay did not reach the boundary",
            Self::ObservedDivergence { .. } => "prefix replay diverged from the recorded prefix",
            Self::ProviderCallsDuringReplay { .. } => {
                "prefix replay invoked a provider instead of replaying exactly"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PrefixForkRefusal {}

impl From<PrefixBoundaryError> for PrefixForkRefusal {
    fn from(error: PrefixBoundaryError) -> Self {
        Self::Boundary(error)
    }
}
