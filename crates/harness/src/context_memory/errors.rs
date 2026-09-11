// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryError {
    Disabled,
    InvalidScope,
    InvalidEntry,
    AuthorityPromotion,
    LineageCycle,
    MissingParent,
    CrossScopeParent,
    FutureParent,
    LineageTooDeep,
    Conflict,
    Capacity,
    PublicationFailed,
    Revoked,
    Expired,
    ProjectionUnavailable,
    InvalidQuery,
    QueryTooLarge,
    TooManyTerms,
    PermissionDenied,
    InvalidProposal,
    ReviewBinding,
    JobConflict,
    JobUnknown,
    BudgetExceeded,
    MandatoryOverflow,
    PinSelection,
    StaleApproval,
    Unsupported,
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Disabled => "memory is disabled",
            Self::InvalidScope => "memory scope is invalid",
            Self::InvalidEntry => "memory entry is invalid",
            Self::AuthorityPromotion => "derived memory cannot claim observed authority",
            Self::LineageCycle => "memory provenance graph contains a cycle",
            Self::MissingParent => "memory provenance parent is missing",
            Self::CrossScopeParent => "memory provenance parent crosses scope",
            Self::FutureParent => "memory provenance parent is newer than its child",
            Self::LineageTooDeep => "memory provenance depth exceeds its bound",
            Self::Conflict => "memory identity conflicts with retained bytes",
            Self::Capacity => "memory capacity is exhausted",
            Self::PublicationFailed => "memory publication failed before atomic swap",
            Self::Revoked => "memory source is revoked",
            Self::Expired => "memory source is expired",
            Self::ProjectionUnavailable => "memory index projection is unavailable",
            Self::InvalidQuery => "memory query is invalid",
            Self::QueryTooLarge => "memory query exceeds its byte bound",
            Self::TooManyTerms => "memory query expands beyond its term bound",
            Self::PermissionDenied => "memory permission is denied",
            Self::InvalidProposal => "summary proposal is invalid",
            Self::ReviewBinding => "summary review binding is invalid",
            Self::JobConflict => "summary job idempotency key conflicts",
            Self::JobUnknown => "summary job outcome is unknown",
            Self::BudgetExceeded => "selection budget is exceeded",
            Self::MandatoryOverflow => "mandatory context exceeds the whole-input bound",
            Self::PinSelection => "pinned memory is not selected",
            Self::StaleApproval => "memory approval is stale",
            Self::Unsupported => "operation is unsupported",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for MemoryError {}
