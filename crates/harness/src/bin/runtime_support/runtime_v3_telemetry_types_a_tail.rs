// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureCode {
    InputBlocked,
    StaleCatalog,
    IllegalAction,
    MissingOperation,
    MalformedDecision,
    ProviderUnavailable,
    ProviderMalformed,
    ProviderClosed,
    Rejected,
    UnknownOutcome,
    Cleanup,
    Configuration,
    Other,
}

impl FailureCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::InputBlocked => "input_blocked",
            Self::StaleCatalog => "stale_catalog",
            Self::IllegalAction => "illegal_action",
            Self::MissingOperation => "missing_operation",
            Self::MalformedDecision => "malformed_decision",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderMalformed => "provider_malformed",
            Self::ProviderClosed => "provider_closed",
            Self::Rejected => "rejected",
            Self::UnknownOutcome => "unknown_outcome",
            Self::Cleanup => "cleanup_failed",
            Self::Configuration => "configuration_failed",
            Self::Other => "other",
        }
    }
}

impl From<&PolicyError> for FailureCode {
    fn from(error: &PolicyError) -> Self {
        match error {
            PolicyError::InputBlocked => Self::InputBlocked,
            PolicyError::StaleCatalog => Self::StaleCatalog,
            PolicyError::IllegalAction => Self::IllegalAction,
            PolicyError::MissingOperation => Self::MissingOperation,
            PolicyError::MalformedDecision => Self::MalformedDecision,
            PolicyError::ProviderUnavailable => Self::ProviderUnavailable,
            PolicyError::ProviderMalformed => Self::ProviderMalformed,
            PolicyError::ProviderClosed => Self::ProviderClosed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryKind {
    Reconnect,
    Reobserve,
    Reconcile,
}

impl RecoveryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Reobserve => "reobserve",
            Self::Reconcile => "reconcile",
        }
    }
}
