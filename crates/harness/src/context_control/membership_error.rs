// SPDX-License-Identifier: MIT

//! Refusal vocabulary for the per-invocation context membership boundary.
//!
//! Split out of [`super::membership`] so the policy vocabulary and the refusal vocabulary
//! can be read, reviewed, and sized independently. Every variant names a gate that refused a
//! dispatch, and [`ContextMembershipError::reason_code`] gives each one a stable code so a
//! caller reports the exact gate that failed rather than a generic provider failure.

use std::fmt::{Display, Formatter};

/// Membership resolution and revalidation failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextMembershipError {
    /// The policy, scope, or draft is malformed.
    InvalidInput(&'static str),
    /// An invocation-scoped item was requested without any wider-scope authorization.
    SiblingScopeLeak { item_id: String },
    /// The caller is authorized, but this specific item is not covered by the authorization.
    WiderScopeNotAuthorized { item_id: String },
    /// The policy tried to exclude an owner prerequisite.
    ProtectedPrerequisiteExcluded { item_id: String },
    /// A requested item or invocation is revoked, expired, unnamed, or digest-mismatched.
    RevokedOrExpired { subject: String },
    /// Owner prerequisites plus policy-added items exceed the bound this invocation may carry.
    MandatoryPinOverflow { bound: usize },
    /// The effective set exceeds the bound this invocation may carry.
    TooManyItems { bound: usize },
    /// Effective absence was requested, but no continuity can execute it yet.
    EffectiveAbsenceUnsupported,
    /// The revalidated policy is not the policy that prepared the bound set.
    PolicyChanged,
    /// The policy could not be canonically encoded.
    Encode,
}

impl Display for ContextMembershipError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::SiblingScopeLeak { item_id } => {
                write!(
                    formatter,
                    "invocation-scoped item {item_id} is not in this scope"
                )
            }
            Self::WiderScopeNotAuthorized { item_id } => {
                write!(formatter, "wider scope does not cover item {item_id}")
            }
            Self::ProtectedPrerequisiteExcluded { item_id } => {
                write!(formatter, "owner prerequisite {item_id} cannot be excluded")
            }
            Self::RevokedOrExpired { subject } => {
                write!(formatter, "revoked or expired: {subject}")
            }
            Self::MandatoryPinOverflow { bound } => write!(
                formatter,
                "owner prerequisites plus pinned items exceed the bound of {bound}"
            ),
            Self::TooManyItems { bound } => {
                write!(formatter, "effective context exceeds the bound of {bound}")
            }
            Self::EffectiveAbsenceUnsupported => {
                formatter.write_str("effective absence is not executable for any continuity yet")
            }
            Self::PolicyChanged => {
                formatter.write_str("membership policy changed since the set was prepared")
            }
            Self::Encode => formatter.write_str("membership policy encoding failed"),
        }
    }
}

impl std::error::Error for ContextMembershipError {}

impl ContextMembershipError {
    /// A stable, precise reason code for this refusal.
    ///
    /// Callers report this code so a refused dispatch names the exact gate that failed rather than
    /// a generic provider failure.
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "context_membership_invalid_input",
            Self::SiblingScopeLeak { .. } => "context_membership_sibling_scope_leak",
            Self::WiderScopeNotAuthorized { .. } => "context_membership_wider_scope_not_authorized",
            Self::ProtectedPrerequisiteExcluded { .. } => {
                "context_membership_protected_prerequisite_excluded"
            }
            Self::RevokedOrExpired { .. } => "context_membership_revoked_or_expired",
            Self::MandatoryPinOverflow { .. } => "context_membership_mandatory_pin_overflow",
            Self::TooManyItems { .. } => "context_membership_too_many_items",
            Self::EffectiveAbsenceUnsupported => "context_membership_effective_absence_unsupported",
            Self::PolicyChanged => "context_membership_policy_changed",
            Self::Encode => "context_membership_encode",
        }
    }
}
