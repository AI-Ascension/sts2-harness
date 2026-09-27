// SPDX-License-Identifier: MIT

//! The witness types a resolved membership decision is published as.
//!
//! Split out of [`super::membership`] so the policy a caller fills in and the witnesses a
//! caller reads back are sized and reviewed independently. Every type here is an *output* of
//! resolution — one typed decision per reference, the effective set, the dispatch view, the
//! revalidation context, and the bound decision. None of them is part of the versioned policy
//! schema, and none of them carries a resolution rule.

use super::membership::{
    AncestorHistoryMode, ContextMembershipScope, MembershipContinuity, MembershipReasonCode,
};
use super::types::ContextItemRef;
use serde::{Deserialize, Serialize};

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
