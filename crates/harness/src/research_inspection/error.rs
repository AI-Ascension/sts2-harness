// SPDX-License-Identifier: MIT

//! Refusals the scoped research-inspection contract can report.

/// A bounded, distinguishable refusal from admitting or serving a research read.
///
/// The variants are the fixed vocabulary a router branches on. A refusal never
/// carries a value, a digest or a field name from the checkpoint, so an ordinary
/// lane cannot learn hidden data by making a request fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchInspectionError {
    /// The request did not match the pinned research schema version.
    Incompatible,
    /// A field reference was empty, oversized or not a plain identifier.
    InvalidField,
    /// A research identity (checkpoint, run, branch or grant) was unusable.
    InvalidScope,
    /// The target did not match the checkpoint, run and branch the grant binds.
    WrongTarget,
    /// The grant was revoked, superseded or already spent by another lane.
    RevokedScope,
    /// The request named a group or field the grant does not admit.
    ProtectedField,
    /// The page bounds were unusable (zero or above the declared maximum).
    InvalidPage,
    /// The grant, or the request's field list, exceeded its declared bound.
    TooManyFields,
}

impl std::fmt::Display for ResearchInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Incompatible => "research inspection schema version is unsupported",
            Self::InvalidField => "research field reference is invalid",
            Self::InvalidScope => "research inspection scope is incomplete or malformed",
            Self::WrongTarget => "research request targets another checkpoint, run or branch",
            Self::RevokedScope => "research inspection grant is revoked or superseded",
            Self::ProtectedField => "research field is outside the admitted scope",
            Self::InvalidPage => "research page bounds are invalid",
            Self::TooManyFields => "research field selection exceeds its declared bound",
        };
        formatter.write_str(label)
    }
}

impl std::error::Error for ResearchInspectionError {}
