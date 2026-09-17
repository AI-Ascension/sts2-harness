// SPDX-License-Identifier: MIT

//! Resolution, scoping, and pre-dispatch gating for per-invocation context membership.
//!
//! This module owns the decision itself: which references one invocation carries, why each was
//! included or excluded, and the gates that must hold before the result may be dispatched. The
//! policy, scope, and error *types* live in the parent module so their public documentation sits
//! with them.

use super::{
    ContextMembershipError, ContextMembershipPolicy, ContextMembershipScope, EffectiveMembership,
    MAX_CONTEXT_ITEMS, MembershipCheckContext, MembershipDecision, MembershipDispatchView,
    MembershipDisposition, MembershipReasonCode, PreparedMembership, SHARED_MEMBERSHIP_KINDS,
};
use crate::context_control::types::{ContextDraft, ContextItem, ContextItemRef};
use crate::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};

fn item_key(reference: &ContextItemRef) -> String {
    format!("{}:{}", reference.item_id, reference.version)
}

fn is_shared_kind(kind: &str) -> bool {
    SHARED_MEMBERSHIP_KINDS.contains(&kind)
}

/// Confirms one requested reference exists, is unchanged, and is in scope.
///
/// `check_expiry` is false for excluded references: removing content that has already expired is
/// legitimate, while *including* it is not.
fn admit_requested<'a>(
    reference: &ContextItemRef,
    registry: &'a BTreeMap<String, ContextItem>,
    policy: &ContextMembershipPolicy,
    caller_scope: &ContextMembershipScope,
    now: u64,
    check_expiry: bool,
) -> Result<&'a ContextItem, ContextMembershipError> {
    let item = registry.get(&item_key(reference)).ok_or_else(|| {
        ContextMembershipError::RevokedOrExpired {
            subject: reference.item_id.clone(),
        }
    })?;
    if item.reference != *reference || sha256_hex(&item.bytes) != reference.sha256 {
        return Err(ContextMembershipError::RevokedOrExpired {
            subject: reference.item_id.clone(),
        });
    }
    if check_expiry && item.expires_at <= now {
        return Err(ContextMembershipError::RevokedOrExpired {
            subject: reference.item_id.clone(),
        });
    }
    if !is_shared_kind(&item.kind) {
        if !policy
            .broader_scope
            .authorized_agent_ids
            .iter()
            .any(|agent| agent == &caller_scope.agent_id)
        {
            return Err(ContextMembershipError::SiblingScopeLeak {
                item_id: reference.item_id.clone(),
            });
        }
        if !policy
            .broader_scope
            .items
            .iter()
            .any(|wider| wider == reference)
        {
            return Err(ContextMembershipError::WiderScopeNotAuthorized {
                item_id: reference.item_id.clone(),
            });
        }
    }
    Ok(item)
}

/// Resolves a policy against one draft and registry.
///
/// This is the pure policy step: it decides membership and records why. Revocation, continuity, and
/// dispatch bounds are enforced by [`prevalidate_and_bind`], which is the entry point a caller
/// preparing an invocation should use.
pub fn resolve_membership(
    policy: &ContextMembershipPolicy,
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
    caller_scope: &ContextMembershipScope,
    now: u64,
) -> Result<EffectiveMembership, ContextMembershipError> {
    if !policy.valid() {
        return Err(ContextMembershipError::InvalidInput(
            "membership policy is invalid",
        ));
    }
    if !caller_scope.valid() {
        return Err(ContextMembershipError::InvalidInput(
            "membership scope is invalid",
        ));
    }
    if policy.base_revision_id != draft.base_revision_id {
        return Err(ContextMembershipError::InvalidInput(
            "membership policy does not belong to this draft revision",
        ));
    }
    let policy_digest = policy.digest()?;

    // Ordered candidate list: the draft selection first, then any policy-added items.
    let mut candidates: Vec<(ContextItemRef, MembershipReasonCode)> = Vec::new();
    let mut seen = BTreeSet::new();
    let inclusion_reason = match policy.disposition {
        MembershipDisposition::Include => MembershipReasonCode::SelectedByDraft,
        MembershipDisposition::Exclude => MembershipReasonCode::SelectedByDraft,
        MembershipDisposition::Inherit => MembershipReasonCode::Inherited,
    };
    for reference in &draft.selected_items {
        if seen.insert((reference.item_id.clone(), reference.version)) {
            candidates.push((reference.clone(), inclusion_reason));
        }
    }
    if policy.disposition == MembershipDisposition::Include {
        for reference in &policy.overrides {
            if seen.insert((reference.item_id.clone(), reference.version)) {
                candidates.push((reference.clone(), MembershipReasonCode::ExplicitInclude));
            }
        }
    }

    let mut included = Vec::new();
    let mut mandatory = Vec::new();
    let mut model_visible = Vec::new();
    let mut decisions = Vec::new();
    for (reference, reason) in &candidates {
        let item = admit_requested(reference, registry, policy, caller_scope, now, true)?;
        if item.protected {
            mandatory.push(reference.clone());
            decisions.push(MembershipDecision {
                reference: reference.clone(),
                included: true,
                reason: MembershipReasonCode::MandatoryPrerequisite,
            });
        } else {
            model_visible.push(reference.clone());
            decisions.push(MembershipDecision {
                reference: reference.clone(),
                included: true,
                reason: *reason,
            });
        }
        included.push(reference.clone());
    }

    // Explicit exclusions are validated too, so this policy cannot be used to probe foreign items.
    let mut excluded = Vec::new();
    if policy.disposition == MembershipDisposition::Exclude {
        for reference in &policy.overrides {
            let item = admit_requested(reference, registry, policy, caller_scope, now, false)?;
            if item.protected {
                return Err(ContextMembershipError::ProtectedPrerequisiteExcluded {
                    item_id: reference.item_id.clone(),
                });
            }
            let key = (reference.item_id.clone(), reference.version);
            if !seen.contains(&key) {
                return Err(ContextMembershipError::InvalidInput(
                    "excluded item is not part of this draft selection",
                ));
            }
            excluded.push(reference.clone());
            decisions.push(MembershipDecision {
                reference: reference.clone(),
                included: false,
                reason: MembershipReasonCode::ExplicitExclude,
            });
        }
        let excluded_keys: BTreeSet<(&str, u64)> = excluded
            .iter()
            .map(|reference| (reference.item_id.as_str(), reference.version))
            .collect();
        included.retain(|reference| {
            !excluded_keys.contains(&(reference.item_id.as_str(), reference.version))
        });
        model_visible.retain(|reference| {
            !excluded_keys.contains(&(reference.item_id.as_str(), reference.version))
        });
        mandatory.retain(|reference| {
            !excluded_keys.contains(&(reference.item_id.as_str(), reference.version))
        });
    }

    // Record what exists but was not requested, so the effective set is fully explained.
    let requested: BTreeSet<(&str, u64)> = candidates
        .iter()
        .map(|(reference, _)| (reference.item_id.as_str(), reference.version))
        .collect();
    for item in registry.values() {
        let key = (item.reference.item_id.as_str(), item.reference.version);
        if !requested.contains(&key) {
            decisions.push(MembershipDecision {
                reference: item.reference.clone(),
                included: false,
                reason: MembershipReasonCode::NotSelected,
            });
        }
    }

    decisions.sort_by(|left, right| {
        (&left.reference.item_id, left.reference.version)
            .cmp(&(&right.reference.item_id, right.reference.version))
    });
    // Pins follow the effective inclusion, never the draft: an excluded item cannot stay pinned,
    // and only `Inherit` reuses the draft's pins at all.
    let mut pins: Vec<String> =
        if policy.disposition == MembershipDisposition::Inherit && policy.inherit_pins {
            draft.pinned_item_ids.clone()
        } else {
            Vec::new()
        };
    pins.retain(|item_id| {
        included
            .iter()
            .any(|reference| reference.item_id == *item_id)
    });
    pins.sort();
    pins.dedup();
    included
        .sort_by(|left, right| (&left.item_id, left.version).cmp(&(&right.item_id, right.version)));
    excluded
        .sort_by(|left, right| (&left.item_id, left.version).cmp(&(&right.item_id, right.version)));
    mandatory
        .sort_by(|left, right| (&left.item_id, left.version).cmp(&(&right.item_id, right.version)));
    model_visible
        .sort_by(|left, right| (&left.item_id, left.version).cmp(&(&right.item_id, right.version)));

    Ok(EffectiveMembership {
        included,
        excluded,
        pins,
        mandatory,
        model_visible,
        decisions,
        policy_digest,
        observation_visible: policy.model_view.observation_visible,
    })
}

fn dispatch_view(
    policy: &ContextMembershipPolicy,
    effective: &EffectiveMembership,
) -> MembershipDispatchView {
    MembershipDispatchView {
        invocation_id: policy.invocation_id.clone(),
        policy_digest: effective.policy_digest.clone(),
        included: effective.included.clone(),
        pins: effective.pins.clone(),
        model_visible: effective.model_visible.clone(),
        decisions: effective.decisions.clone(),
        observation_visible: effective.observation_visible,
    }
}

/// Enforces the gates that must hold before this invocation is dispatched.
fn enforce_dispatch_gates(
    policy: &ContextMembershipPolicy,
    effective: &EffectiveMembership,
    check: &MembershipCheckContext,
    bound: usize,
    draft: &ContextDraft,
) -> Result<(), ContextMembershipError> {
    if check
        .revoked_invocation_ids
        .iter()
        .any(|id| id == &policy.invocation_id)
    {
        return Err(ContextMembershipError::RevokedOrExpired {
            subject: policy.invocation_id.clone(),
        });
    }
    for reference in &effective.included {
        if check
            .revoked_item_ids
            .iter()
            .any(|id| id == &reference.item_id)
        {
            return Err(ContextMembershipError::RevokedOrExpired {
                subject: reference.item_id.clone(),
            });
        }
    }
    // Fail closed. Effective absence is not executable for any continuity today: the provider
    // request schema (`sts2.exo-decision-v1`) makes `observation` a required, state-bound field and
    // its validation requires the observation to carry the same `state_id`/`generation` and to be
    // the source of `legal_action_ids`, so a request that omits the observation cannot be built,
    // let alone validated. An admitted `observation_visible: false` would therefore ship a
    // `dispatch_view` that says the observation is hidden while the prepared bytes still contain
    // it. Refusing here keeps the selector truthful until a versioned omission wireform exists.
    if !policy.model_view.observation_visible {
        return Err(ContextMembershipError::EffectiveAbsenceUnsupported);
    }
    let draft_keys: BTreeSet<(&str, u64)> = draft
        .selected_items
        .iter()
        .map(|reference| (reference.item_id.as_str(), reference.version))
        .collect();
    let additional = effective
        .included
        .iter()
        .filter(|reference| !draft_keys.contains(&(reference.item_id.as_str(), reference.version)))
        .count();
    // Owner prerequisites are non-negotiable, so they are accounted for first: if they alone exceed
    // the bound, no selector choice could have made this invocation admissible.
    if effective.mandatory.len() > bound {
        return Err(ContextMembershipError::MandatoryPinOverflow { bound });
    }
    if effective.included.len() > bound {
        // Attribute the overflow to its cause: a non-negotiable prerequisite or a policy-added item
        // is a mandatory/pin overflow, while an oversized plain draft selection is not.
        if effective.mandatory.is_empty() && additional == 0 {
            return Err(ContextMembershipError::TooManyItems { bound });
        }
        return Err(ContextMembershipError::MandatoryPinOverflow { bound });
    }
    Ok(())
}

/// Resolves, gates, and binds one invocation's membership.
///
/// This is the entry point a caller uses *before* claiming a dispatch. It fails closed on revoked or
/// expired content, on owner prerequisites the policy tried to exclude, on bounds, and on effective
/// absence a persistent adapter cannot execute.
pub fn prevalidate_and_bind(
    policy: &ContextMembershipPolicy,
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
    now: u64,
    check: &MembershipCheckContext,
    max_items: usize,
) -> Result<PreparedMembership, ContextMembershipError> {
    let effective = resolve_membership(policy, draft, registry, &check.caller_scope, now)?;
    let bound = max_items.min(MAX_CONTEXT_ITEMS);
    enforce_dispatch_gates(policy, &effective, check, bound, draft)?;
    let view = dispatch_view(policy, &effective);
    Ok(PreparedMembership {
        policy_digest: effective.policy_digest.clone(),
        effective,
        dispatch_view: view,
    })
}

impl EffectiveMembership {
    /// Re-derives this set from the current registry and refuses anything that moved since
    /// preparation.
    ///
    /// Revalidation never mutates the bound set. An unpin or an exclusion therefore changes only the
    /// *next* prepared input, while this already-prepared set and its digest stay reproducible.
    pub fn revalidate(
        &self,
        policy: &ContextMembershipPolicy,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        check: &MembershipCheckContext,
        now: u64,
        max_items: usize,
    ) -> Result<MembershipDispatchView, ContextMembershipError> {
        let refreshed = resolve_membership(policy, draft, registry, &check.caller_scope, now)?;
        if refreshed.policy_digest != self.policy_digest {
            return Err(ContextMembershipError::PolicyChanged);
        }
        let bound = max_items.min(MAX_CONTEXT_ITEMS);
        enforce_dispatch_gates(policy, &refreshed, check, bound, draft)?;
        if refreshed.model_visible != self.model_visible || refreshed.included != self.included {
            return Err(ContextMembershipError::PolicyChanged);
        }
        Ok(dispatch_view(policy, &refreshed))
    }
}
