// SPDX-License-Identifier: MIT

//! Rejection reasons for transition records and traces.

/// Rejection reasons for transition records and traces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    /// A record field is empty, too long, or contains a NUL separator.
    InvalidRecord,
    /// A digest field is not the expected namespace.
    InvalidDigest,
    /// A trace has no profile, no records, or too many records.
    InvalidTrace,
    /// Ordinals do not strictly increase.
    NonMonotonicOrdinal,
    /// Genesis or a previous commitment does not bind the predecessor.
    BrokenCommitmentChain,
}
