// SPDX-License-Identifier: MIT

//! Wires the per-invocation membership boundary into the production render path.
//!
//! Until this module existed, [`resolve_membership`](super::resolve_membership) and
//! [`prevalidate_and_bind`] had no production caller: the live managed
//! render always published every `draft.selected_items` reference, so a per-invocation inclusion
//! policy could not actually change application bytes.
//!
//! This module is the single seam that closes that gap. It resolves and gates the invocation's
//! policy *before* any provider bytes exist, projects the effective `model_visible` subset onto the
//! draft, and then delegates to [`ContextRenderer::enabled_at_with_limits`] so the selected owner
//! limits still compose with (narrow) the membership bound rather than being replaced by it.
//!
//! When no policy is in force the draft is rendered unchanged, so an invocation without a policy
//! keeps today's exact bytes. Protected owner prerequisites are retained in owner state and
//! suppressed from model-visible input; they are never published, and a pin that would name one is
//! dropped with them.

use super::membership::{
    ContextMembershipError, ContextMembershipPolicy, ContextMembershipScope,
    MembershipCheckContext, MembershipContinuity, PreparedMembership, prevalidate_and_bind,
};
use super::render::{ContextRenderError, ContextRenderLimits, ContextRenderer, ManagedRenderInput};
use super::types::ContextBoundary;
use super::types::{ContextDraft, ContextSourceDocument};
use crate::exo::ExoConfig;

/// A membership refusal, kept distinct from a render refusal so a caller can report the precise
/// gate that failed before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MembershipRenderError {
    /// Membership resolution or its pre-dispatch gates refused this invocation.
    Membership(ContextMembershipError),
    /// The projected draft could not be rendered under the selected owner limits.
    Render(ContextRenderError),
}

impl std::fmt::Display for MembershipRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Membership(error) => write!(formatter, "{error}"),
            Self::Render(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for MembershipRenderError {}

/// The invocation scope a membership decision is made on behalf of.
///
/// Only identities the boundary already carries are used. The live render seam has no reachable
/// branch identity (durable branch continuation is selected by a different entry point and is never
/// projected onto the render source), so `branch_id` is encoded as absent rather than fabricated.
#[must_use]
pub fn membership_scope_from_boundary(boundary: &ContextBoundary) -> ContextMembershipScope {
    ContextMembershipScope {
        run_id: boundary.run_id.clone(),
        episode_id: boundary.episode_id.clone(),
        agent_id: boundary.agent_id.clone(),
        branch_id: None,
    }
}

/// Builds the revalidation context for one live invocation.
///
/// The scope and both epochs come from the admitted boundary; continuity is derived from the
/// selected binding. Revocation lists stay empty because no revocation registry is reachable from
/// the render seam: claiming a revocation that cannot be observed would be a fabrication. Content
/// expiry is still enforced independently from each item's `expires_at`.
#[must_use]
pub fn membership_check_from_boundary(
    boundary: &ContextBoundary,
    continuity: MembershipContinuity,
) -> MembershipCheckContext {
    MembershipCheckContext {
        caller_scope: membership_scope_from_boundary(boundary),
        continuity,
        generation: boundary.generation,
        controller_epoch: boundary.controller_epoch,
        gate_epoch: boundary.gate_epoch,
        revoked_item_ids: Vec::new(),
        revoked_invocation_ids: Vec::new(),
    }
}

/// Everything one invocation needs to resolve, gate, project, and render its membership.
///
/// The draft travels with the document's own item registry, so a caller cannot pair a draft with a
/// registry that does not describe it. `limits` is the selected owner bound; membership composition
/// narrows it, so an effective set the owner cannot accept is refused before any inference.
pub struct MembershipRenderRequest<'a> {
    pub boundary: &'a ContextBoundary,
    pub request: ManagedRenderInput,
    pub document: &'a ContextSourceDocument,
    pub config: &'a ExoConfig,
    pub now: u64,
    pub limits: &'a ContextRenderLimits,
    /// The invocation's finished policy, already bound to its exact invocation identity. `None`
    /// renders today's exact bytes.
    pub policy: Option<&'a ContextMembershipPolicy>,
    /// The continuity the selected binding can actually execute for this invocation.
    pub continuity: MembershipContinuity,
}

/// Resolves, gates, and projects one invocation's membership, then renders it.
pub fn render_with_membership(
    input: MembershipRenderRequest<'_>,
) -> Result<(super::render::PreparedContext, Option<PreparedMembership>), MembershipRenderError> {
    let MembershipRenderRequest {
        boundary,
        request,
        document,
        config,
        now,
        limits,
        policy,
        continuity,
    } = input;
    let Some(policy) = policy else {
        // No policy in force: preserve today's exact bytes.
        return ContextRenderer::enabled_at_with_limits(
            boundary,
            request,
            &document.draft,
            &document.items,
            config,
            now,
            limits,
        )
        .map(|prepared| (prepared, None))
        .map_err(MembershipRenderError::Render);
    };
    let check = membership_check_from_boundary(boundary, continuity);
    let prepared = prevalidate_and_bind(
        policy,
        &document.draft,
        &document.items,
        now,
        &check,
        limits.max_items,
    )
    .map_err(MembershipRenderError::Membership)?;
    let projected = project_draft(&document.draft, &prepared);
    ContextRenderer::enabled_at_with_limits(
        boundary,
        request,
        &projected,
        &document.items,
        config,
        now,
        limits,
    )
    .map(|rendered| (rendered, Some(prepared)))
    .map_err(MembershipRenderError::Render)
}

/// Projects an effective membership onto a draft for rendering.
///
/// Model-visible items become the selection; retained prerequisites stay in owner state and out of
/// the published bytes. Pins are narrowed to model-visible ids so a pin can never name a
/// prerequisite the model cannot see.
fn project_draft(draft: &ContextDraft, prepared: &PreparedMembership) -> ContextDraft {
    let effective = &prepared.effective;
    let mut projected = draft.clone();
    projected.selected_items = effective.model_visible.clone();
    let visible: std::collections::BTreeSet<&str> = effective
        .model_visible
        .iter()
        .map(|reference| reference.item_id.as_str())
        .collect();
    projected.pinned_item_ids = effective
        .pins
        .iter()
        .filter(|item_id| visible.contains(item_id.as_str()))
        .cloned()
        .collect();
    // Notes are ancestor annotations rendered directly from the registry, so they are narrowed to
    // model-visible items exactly like pins. Under current-observation-only, no item is
    // model-visible, so no ancestor note is published either.
    projected
        .notes
        .retain(|note| visible.contains(note.reference.item_id.as_str()));
    projected
}
