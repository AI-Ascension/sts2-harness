// SPDX-License-Identifier: MIT

//! #118 AC3 — late parent/sibling provider responses and stale approved proposals cannot dispatch
//! to a child, even when the restored public state matches.
//!
//! The prepared-dispatch boundary binds an approval to every axis that could change what its exact
//! bytes mean, including the authority epochs and the per-branch profile/auth/history digests. A
//! late response from a parent branch, or a sibling branch's approval, therefore drifts before any
//! write even when the child restores the same public state digest.
//!
//! Synthetic fixtures only: application-controlled bytes, an in-memory recording sink, and no
//! provider, host or game process.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "prepared_dispatch/dispatch_fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use fixtures::{approved, base_fences, capture, digest};
use sts2_harness::context_capture::{
    CaptureRecordingPort, DispatchError, DispatchFences, DriftAxis, PreparedDispatchController,
};

/// The public state the child restores. Every scenario below shares it, so a refusal cannot be
/// attributed to a changed `state_digest`.
fn restored_public_state() -> String {
    digest("restored-public-state")
}

/// The child's branch identity axes: shared by the child and by a late parent response so only the
/// authority epoch can distinguish them.
fn child_identity(fences: &mut DispatchFences) {
    fences.state_digest = restored_public_state();
    fences.profile_digest = digest("child-profile");
    fences.auth_digest = digest("child-auth");
    fences.history_digest = digest("child-history");
}

/// A parent branch's approval fences: same public state and branch identity, parent authority
/// epochs (the `base_fences` defaults). Only the epoch axes distinguish it from the child.
fn late_parent_fences() -> DispatchFences {
    let mut fences = base_fences("exo");
    child_identity(&mut fences);
    fences
}

/// A sibling branch's approval fences: same public state, sibling-branch identity.
fn sibling_branch_fences() -> DispatchFences {
    let mut fences = base_fences("exo");
    fences.state_digest = restored_public_state();
    fences.profile_digest = digest("sibling-profile");
    fences.auth_digest = digest("sibling-auth");
    fences.history_digest = digest("sibling-history");
    fences
}

/// The child's current authority: same restored public state, advanced epochs and child identity.
fn child_current_fences() -> DispatchFences {
    let mut fences = base_fences("exo");
    child_identity(&mut fences);
    fences.controller_epoch = 12;
    fences.gate_epoch = 8;
    fences.lease_epoch = 6;
    fences.revocation_epoch = 3;
    fences
}

/// AC3: a late parent response, held against the parent's epochs, drifts on an epoch axis and
/// writes nothing into the child — despite the restored public state matching.
#[test]
fn late_parent_response_cannot_dispatch_to_a_child() {
    let mut controller = PreparedDispatchController::new();
    let parent = late_parent_fences();
    let child = child_current_fences();
    // The only difference between the parent approval and the child's current authority is the
    // authority epoch, so the state digest matching is not what admits or refuses the write.
    assert_eq!(parent.state_digest, child.state_digest);
    assert_ne!(parent.controller_epoch, child.controller_epoch);

    controller
        .draft("dispatch-late-parent", approved("exo"), parent)
        .expect("draft");
    controller
        .commit("dispatch-late-parent", &late_parent_fences())
        .expect("commit");

    let mut capture = capture();
    let mut port = CaptureRecordingPort::new(&mut capture);
    let refused = controller.resume("dispatch-late-parent", &child, &mut port);
    assert_eq!(
        refused,
        Err(DispatchError::Drift(DriftAxis::Controller)),
        "a late parent approval must drift on the authority epoch"
    );
    assert_eq!(port.write_attempts(), 0, "no write may reach the boundary");
    assert_eq!(controller.boundary_writes(), 0);
    assert_eq!(controller.gameplay_effects(), 0);
    // A refused approval is terminal: it cannot be retried into the child.
    assert_eq!(
        controller.resume("dispatch-late-parent", &child, &mut port),
        Err(DispatchError::Stale)
    );
    assert_eq!(port.write_attempts(), 0);
}

/// AC3: a sibling branch's approved proposal, held with the sibling's identity, drifts on the
/// branch identity axes and writes nothing into the child — despite the restored public state
/// matching.
#[test]
fn sibling_approved_proposal_cannot_dispatch_to_a_child() {
    let mut controller = PreparedDispatchController::new();
    let sibling = sibling_branch_fences();
    let child = child_current_fences();
    assert_eq!(sibling.state_digest, child.state_digest);
    assert_ne!(sibling.profile_digest, child.profile_digest);

    controller
        .draft("dispatch-sibling", approved("exo"), sibling)
        .expect("draft");
    controller
        .commit("dispatch-sibling", &sibling_branch_fences())
        .expect("commit");

    let mut capture = capture();
    let mut port = CaptureRecordingPort::new(&mut capture);
    let refused = controller.resume("dispatch-sibling", &child, &mut port);
    assert_eq!(
        refused,
        Err(DispatchError::Drift(DriftAxis::Profile)),
        "a sibling approval must drift on the branch identity"
    );
    assert_eq!(port.write_attempts(), 0);
    assert_eq!(controller.boundary_writes(), 0);
}

/// Control: the child's own fresh approval under the child's current authority does dispatch, so
/// the refusals above are caused by the late/sibling origin rather than a broken controller.
#[test]
fn child_approval_under_current_authority_dispatches() {
    let mut controller = PreparedDispatchController::new();
    let child = child_current_fences();
    controller
        .draft("dispatch-child", approved("exo"), child.clone())
        .expect("draft");
    controller
        .commit("dispatch-child", &child_current_fences())
        .expect("commit");

    let mut capture = capture();
    let mut port = CaptureRecordingPort::new(&mut capture);
    let receipt = controller
        .resume("dispatch-child", &child, &mut port)
        .expect("child approval dispatches");
    assert_eq!(receipt.boundary_writes, 1);
    assert_eq!(controller.boundary_writes(), 1);
}
