// SPDX-License-Identifier: MIT

//! Typed refusals for logical-invocation lifetime consumption (issue #111).

use std::fmt::{Display, Formatter};

/// Lifetime-scope construction, applicability, admission, and reconciliation failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextLifetimeError {
    /// The schema, scope identity, or declared window is malformed.
    InvalidInput,
    /// An owner identity is missing or not a valid identifier.
    InvalidOwner,
    /// A next-N bound is zero, which would make the scope unusable.
    EmptyBound,
    /// The wall-clock ceiling precedes the instant the scope was issued.
    InvertedCeiling,
    /// No scope is recorded under this identity.
    UnknownScope { scope_id: String },
    /// A scope identity was reused for a different owner, bound, or issuance.
    RepeatedScope { scope_id: String },
    /// The invocation belongs to a different agent, branch, episode, or run.
    SiblingScopeRefused { scope_id: String },
    /// The scope's declared capacity is already consumed.
    Exhausted { scope_id: String },
    /// The wall-clock ceiling has passed.
    Expired { scope_id: String },
    /// Reconciliation named an invocation that is not held.
    NotHeld { invocation_id: String },
    /// Reconciliation tried to move an already settled invocation to a different settlement.
    AlreadySettled { invocation_id: String },
    /// A failpoint stopped the operation before it became durable.
    InterruptedBeforeAdmission,
    /// A failpoint stopped the operation after it became durable.
    InterruptedAfterAdmission,
    /// The manifest could not be canonically encoded.
    Encode,
}

impl Display for ContextLifetimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput => {
                formatter.write_str("lifetime scope schema, identity, or window is invalid")
            }
            Self::InvalidOwner => formatter.write_str("lifetime scope owner identity is invalid"),
            Self::EmptyBound => {
                formatter.write_str("lifetime scope next-N bound must be at least one")
            }
            Self::InvertedCeiling => {
                formatter.write_str("lifetime scope wall-clock ceiling precedes its issuance")
            }
            Self::UnknownScope { scope_id } => {
                write!(formatter, "lifetime scope {scope_id} is not recorded")
            }
            Self::RepeatedScope { scope_id } => write!(
                formatter,
                "lifetime scope {scope_id} is already bound to a different owner or window"
            ),
            Self::SiblingScopeRefused { scope_id } => write!(
                formatter,
                "lifetime scope {scope_id} does not authorize this agent, branch, episode, or run"
            ),
            Self::Exhausted { scope_id } => {
                write!(
                    formatter,
                    "lifetime scope {scope_id} has no remaining capacity"
                )
            }
            Self::Expired { scope_id } => {
                write!(
                    formatter,
                    "lifetime scope {scope_id} is past its wall-clock ceiling"
                )
            }
            Self::NotHeld { invocation_id } => write!(
                formatter,
                "logical invocation {invocation_id} is not held for reconciliation"
            ),
            Self::AlreadySettled { invocation_id } => write!(
                formatter,
                "logical invocation {invocation_id} is already settled and cannot change settlement"
            ),
            Self::InterruptedBeforeAdmission => {
                formatter.write_str("dispatch admission was interrupted before it became durable")
            }
            Self::InterruptedAfterAdmission => {
                formatter.write_str("dispatch admission was interrupted after it became durable")
            }
            Self::Encode => formatter.write_str("lifetime manifest encoding failed"),
        }
    }
}

impl std::error::Error for ContextLifetimeError {}

impl ContextLifetimeError {
    /// A stable, precise reason code for this refusal.
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "context_lifetime_invalid_input",
            Self::InvalidOwner => "context_lifetime_invalid_owner",
            Self::EmptyBound => "context_lifetime_empty_bound",
            Self::InvertedCeiling => "context_lifetime_inverted_ceiling",
            Self::UnknownScope { .. } => "context_lifetime_unknown_scope",
            Self::RepeatedScope { .. } => "context_lifetime_repeated_scope",
            Self::SiblingScopeRefused { .. } => "context_lifetime_sibling_scope_refused",
            Self::Exhausted { .. } => "context_lifetime_exhausted",
            Self::Expired { .. } => "context_lifetime_expired",
            Self::NotHeld { .. } => "context_lifetime_not_held",
            Self::AlreadySettled { .. } => "context_lifetime_already_settled",
            Self::InterruptedBeforeAdmission => "context_lifetime_interrupted_before_admission",
            Self::InterruptedAfterAdmission => "context_lifetime_interrupted_after_admission",
            Self::Encode => "context_lifetime_encode",
        }
    }
}
