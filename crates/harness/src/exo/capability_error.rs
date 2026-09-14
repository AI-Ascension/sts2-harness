// SPDX-License-Identifier: MIT

//! Fail-closed preflight errors for the Exo capability contract.

/// Typed, closed preflight error. Every variant is a fail-closed outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoPreflightError {
    Malformed,
    UnknownField,
    MissingField,
    UnsupportedSchema,
    UnsupportedContract,
    UnsupportedPlatform,
    WrongRevision,
    SwappedPackage,
    UnsupportedDecisionKind,
    UnsupportedProjection,
    UnsupportedContextMode,
    UnsupportedEvidence,
    LimitExceeded,
    UnsupportedValue,
}

impl std::fmt::Display for ExoPreflightError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "Exo capability descriptor is not valid JSON",
            Self::UnknownField => "Exo capability descriptor contains an unknown field",
            Self::MissingField => "Exo capability descriptor is missing a required field",
            Self::UnsupportedSchema => "Exo capability schema is unsupported",
            Self::UnsupportedContract => "Exo bridge contract version is unsupported",
            Self::UnsupportedPlatform => "Exo platform is unsupported",
            Self::WrongRevision => "Exo provider revision does not match the approved revision",
            Self::SwappedPackage => "Exo package digest does not match the approved bytes",
            Self::UnsupportedDecisionKind => "Exo decision kind is unsupported",
            Self::UnsupportedProjection => "Exo projection is unsupported",
            Self::UnsupportedContextMode => "Exo context mode is unsupported",
            Self::UnsupportedEvidence => "Exo evidence state is unsupported",
            Self::LimitExceeded => "Exo declared limit exceeds the contract bound",
            Self::UnsupportedValue => "Exo capability descriptor contains an invalid value",
        })
    }
}

impl std::error::Error for ExoPreflightError {}
