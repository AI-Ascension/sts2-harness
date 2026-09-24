// SPDX-License-Identifier: MIT

//! Per-invocation context membership policy.
//!
//! Until this module existed, one owner-issued [`ContextDraft`] decided what entered every model
//! request. Two invocations could not disagree about inclusion without rewriting a persisted
//! revision, nothing recorded *why* an item was included or excluded, and there was no way to keep a
//! mandatory host prerequisite in the owner's state while omitting it from model-visible input.
//!
//! A [`ContextMembershipPolicy`] is the versioned, per-invocation input to that decision. It is
//! resolved into an [`EffectiveMembership`] that names every included and excluded reference
//! together with a typed reason, and binds the whole decision with a policy digest, so a later
//! invocation can prove it is acting on the same policy rather than on a silently widened one.
//!
//! ## Scope
//!
//! Item identity carries no per-invocation marker. Scope is expressed by `ContextItem::kind`:
//! the shared kinds in [`SHARED_MEMBERSHIP_KINDS`] are episode-independent and carryable, while any
//! other kind is owned by one invocation and crossable only through an explicit authorized wider
//! scope. Unknown kinds are therefore default-deny rather than default-allow.
//!
//! ## What this deliberately does not do
//!
//! A membership policy selects which already-collected items become *model-visible context*. It
//! cannot erase what a persistent provider adapter already received. Effective absence of an
//! observation is therefore **refused**, for every continuity, until an omission wireform exists
//! that the render path actually consumes; see [`MembershipContinuity`].

use super::types::{ContextItemRef, MAX_CONTEXT_ITEMS, valid_id};
use crate::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

#[path = "membership_resolution.rs"]
mod resolution;
#[path = "membership_selector.rs"]
mod selector;

pub use resolution::{prevalidate_and_bind, resolve_membership};
pub use selector::ContextMembershipSelector;

/// Schema identity for the versioned per-invocation membership policy.
pub const CONTEXT_MEMBERSHIP_POLICY_SCHEMA: &str = "ascension.context-control.membership.v1";
/// Upper bound on the agents an owner may name in one wider-scope authorization.
pub const MAX_MEMBERSHIP_AUTHORIZED_AGENTS: usize = 16;
/// Upper bound on the individual items one wider-scope authorization may name.
pub const MAX_MEMBERSHIP_WIDER_ITEMS: usize = 16;

/// Item kinds that are episode-independent and therefore in scope for every invocation.
///
/// A kind outside this list is owned by a single invocation's agent/episode and is admissible only
/// through [`ContextMembershipBroaderScope`].
pub const SHARED_MEMBERSHIP_KINDS: [&str; 3] = ["history", "strategy", "objective"];

/// How this invocation derives its effective set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipDisposition {
    /// Start from the draft selection and add the policy's own items.
    Include,
    /// Start from the draft selection and remove the policy's own items.
    Exclude,
    /// Keep the draft selection unchanged; optionally reuse its pins.
    Inherit,
}

/// The continuity a selected provider adapter can actually execute.
///
/// A policy selector cannot erase history an opaque persistent adapter already holds, so effective
/// absence was never executable under [`MembershipContinuity::OpaquePersistent`]. It is **also not
/// executable under [`MembershipContinuity::Stateless`] yet**: the managed render path builds the
/// provider request from an input that always carries the observation, so admitting absence there
/// would report `observation_visible == false` while still shipping the observation. Both
/// continuities are refused until the omission is implemented in the bytes, which keeps the gate's
/// verdict identical to what the provider actually receives.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipContinuity {
    /// Verified stateless, fresh, or reconstructed execution for this invocation.
    ///
    /// Absence is still refused: the render path has no omission wireform yet.
    Stateless,
    /// A persistent adapter whose history this invocation cannot reconstitute.
    OpaquePersistent,
}

impl MembershipContinuity {
    /// Derives the executable continuity from the invocation's selected binding.
    ///
    /// A binding that advertises provider-session continuity keeps provider-side history this
    /// invocation cannot reconstitute, so effective absence is not executable and the result is
    /// [`MembershipContinuity::OpaquePersistent`]. A binding that does not is fresh for every
    /// invocation, so the result is [`MembershipContinuity::Stateless`]; effective absence is
    /// refused for that continuity too until the render path can omit the observation from the
    /// bytes it sends.
    #[must_use]
    pub fn from_provider_session_continuity(provider_session_continuity: bool) -> Self {
        if provider_session_continuity {
            Self::OpaquePersistent
        } else {
            Self::Stateless
        }
    }
}

/// Why one included or excluded reference acquired its outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipReasonCode {
    /// The draft selected this item and this policy left it in.
    SelectedByDraft,
    /// The policy added this item on top of the draft selection.
    ExplicitInclude,
    /// The policy removed this item from the draft selection.
    ExplicitExclude,
    /// The draft selection was reused unchanged.
    Inherited,
    /// An owner prerequisite retained for legality and host state but hidden from model input.
    MandatoryPrerequisite,
    /// Present in the registry but not requested by this invocation.
    NotSelected,
}

/// The invocation identity a membership decision is made on behalf of.
///
/// `branch_id` is optional because a branch identity is not always reachable. The live managed
/// render seam is driven by an admitted workflow run, its episode and its agent; durable branch
/// continuation is selected by the separate runtime entry point and is never projected onto the
/// context render source. Absence is therefore encoded as `None` rather than filled with a
/// fabricated id, so scope enforcement is honestly agent-and-kind based for branchless
/// invocations. Real per-branch isolation remains owned by its own work item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMembershipScope {
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
}

impl ContextMembershipScope {
    /// Validates every declared identity.
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.run_id)
            && valid_id(&self.episode_id)
            && valid_id(&self.agent_id)
            && self
                .branch_id
                .as_ref()
                .is_none_or(|branch| valid_id(branch))
    }
}

/// The explicit capability that lets one invocation reach beyond its own scope.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMembershipBroaderScope {
    /// Agents permitted to carry the listed items.
    pub authorized_agent_ids: Vec<String>,
    /// The individual invocation-scoped items those agents may carry.
    pub items: Vec<ContextItemRef>,
}

/// Which parts of the collected context the model is allowed to see.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextModelView {
    /// Whether the observation may appear in model-visible input.
    pub observation_visible: bool,
}

/// How much reconstructed ancestor history one branch invocation carries.
///
/// This is the branch-context axis requested by issue #118: a child may either carry the permitted
/// ancestor items selected through its fork occurrence, or run in an explicitly chosen
/// current-observation-only mode that omits all ancestor history while still carrying the live
/// observation. It is orthogonal to [`ContextModelView::observation_visible`]: this axis controls
/// the *ancestor items*, that axis controls the *observation*, so an invocation can be
/// current-observation-only without ever claiming effective absence of the observation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AncestorHistoryMode {
    /// Carry the permitted ancestor history selected through the fork occurrence. The default.
    #[default]
    ThroughFork,
    /// Omit all ancestor history; carry only the current observation.
    CurrentObservationOnly,
}

impl AncestorHistoryMode {
    /// Stable wire label for this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThroughFork => "through_fork",
            Self::CurrentObservationOnly => "current_observation_only",
        }
    }
}

impl ContextModelView {
    /// The default view: the observation is visible.
    #[must_use]
    pub const fn visible() -> Self {
        Self {
            observation_visible: true,
        }
    }
}

/// One versioned, owner-issued inclusion policy for a single invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMembershipPolicy {
    pub schema: String,
    pub invocation_id: String,
    pub base_revision_id: String,
    pub disposition: MembershipDisposition,
    #[serde(default)]
    pub overrides: Vec<ContextItemRef>,
    #[serde(default)]
    pub inherit_pins: bool,
    #[serde(default)]
    pub broader_scope: ContextMembershipBroaderScope,
    pub model_view: ContextModelView,
    /// How much reconstructed ancestor history this invocation carries.
    #[serde(default)]
    pub ancestor_history: AncestorHistoryMode,
}

impl ContextMembershipPolicy {
    /// Validates schema, identities, bounds, uniqueness, and disposition consistency.
    #[must_use]
    pub fn valid(&self) -> bool {
        if self.schema != CONTEXT_MEMBERSHIP_POLICY_SCHEMA
            || !valid_id(&self.invocation_id)
            || !valid_id(&self.base_revision_id)
            || self.overrides.len() > MAX_CONTEXT_ITEMS
            || self.broader_scope.items.len() > MAX_MEMBERSHIP_WIDER_ITEMS
            || self.broader_scope.authorized_agent_ids.len() > MAX_MEMBERSHIP_AUTHORIZED_AGENTS
        {
            return false;
        }
        let mut override_ids = BTreeSet::new();
        if !self
            .overrides
            .iter()
            .all(|reference| reference.valid() && override_ids.insert(reference.item_id.as_str()))
        {
            return false;
        }
        let mut agents = BTreeSet::new();
        if !self
            .broader_scope
            .authorized_agent_ids
            .iter()
            .all(|agent| valid_id(agent) && agents.insert(agent.as_str()))
        {
            return false;
        }
        let mut wider = BTreeSet::new();
        if !self.broader_scope.items.iter().all(|reference| {
            reference.valid() && wider.insert((&reference.item_id, reference.version))
        }) {
            return false;
        }
        let wider_scope_declared = !self.broader_scope.items.is_empty() || !agents.is_empty();
        match self.disposition {
            MembershipDisposition::Include => !self.inherit_pins,
            // `Exclude` may name a wider scope: removing a cross-invocation item still requires the
            // explicit authorization that named it.
            MembershipDisposition::Exclude => !self.inherit_pins,
            // `Inherit` reuses the draft exactly, so it may not widen anything.
            MembershipDisposition::Inherit => self.overrides.is_empty() && !wider_scope_declared,
        }
    }

    /// Stable digest over the canonical encoding of this policy.
    ///
    /// Any field change produces a different digest, so a later invocation can prove it revalidated
    /// the same policy rather than a silently widened one.
    pub fn digest(&self) -> Result<String, ContextMembershipError> {
        let encoded = serde_json::to_vec(self).map_err(|_| ContextMembershipError::Encode)?;
        Ok(sha256_hex(&encoded))
    }
}

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
