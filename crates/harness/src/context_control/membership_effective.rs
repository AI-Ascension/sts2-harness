// SPDX-License-Identifier: MIT

//! The effective set a membership policy resolves to, and the typed refusals that can
//! stop it.
//!
//! These are the *results* of a decision rather than the policy that authorises it: the
//! per-reference outcomes, the resolved effective set, the model-visible projection handed
//! to a dispatcher, and the revalidation state a prepared set is checked against. The
//! policy, scope, and reason-code vocabulary those results are built from stay in the
//! parent module so their public documentation sits with them.

use super::{
    AncestorHistoryMode, ContextMembershipScope, MembershipContinuity, MembershipReasonCode,
};
use crate::context_control::types::ContextItemRef;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// One typed outcome for one reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipDecision {
    pub reference: ContextItemRef,
    pub included: bool,
    pub reason: MembershipReasonCode,
}

/// The resolved effective set, its reasons, and the policy that produced it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveMembership {
    /// Every reference retained for owner legality and host state.
    pub included: Vec<ContextItemRef>,
    /// Every reference this policy explicitly removed.
    pub excluded: Vec<ContextItemRef>,
    /// The owner pins that remain bound to this invocation, ordered by item id.
    pub pins: Vec<String>,
    /// The retained owner prerequisites that must stay out of model-visible input.
    pub mandatory: Vec<ContextItemRef>,
    /// The subset of `included` that may appear in model-visible input.
    pub model_visible: Vec<ContextItemRef>,
    /// One typed outcome per reference considered, ordered by `(item_id, version)`.
    pub decisions: Vec<MembershipDecision>,
    /// Digest of the policy that produced this set.
    pub policy_digest: String,
    /// Whether the observation may appear in model-visible input.
    pub observation_visible: bool,
    /// The ancestor-history mode this set was resolved under.
    #[serde(default)]
    pub ancestor_history: AncestorHistoryMode,
}

/// The model-visible projection plus its witness. Carries no item content, so a caller cannot widen
/// retention from it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipDispatchView {
    pub invocation_id: String,
    pub policy_digest: String,
    pub included: Vec<ContextItemRef>,
    pub pins: Vec<String>,
    pub model_visible: Vec<ContextItemRef>,
    pub decisions: Vec<MembershipDecision>,
    pub observation_visible: bool,
    /// The ancestor-history mode this dispatch view was resolved under.
    #[serde(default)]
    pub ancestor_history: AncestorHistoryMode,
}

/// The current invocation state a revalidation is checked against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipCheckContext {
    pub caller_scope: ContextMembershipScope,
    pub continuity: MembershipContinuity,
    pub generation: u64,
    pub controller_epoch: u64,
    pub gate_epoch: u64,
    /// Item ids revoked since preparation.
    pub revoked_item_ids: Vec<String>,
    /// Invocation ids revoked since preparation.
    pub revoked_invocation_ids: Vec<String>,
}

/// A bound decision ready to be dispatched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedMembership {
    pub policy_digest: String,
    pub effective: EffectiveMembership,
    pub dispatch_view: MembershipDispatchView,
}

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
