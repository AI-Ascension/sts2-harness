// SPDX-License-Identifier: MIT

//! The repository's existing branch retention, applied to a retained history.
//!
//! A branch the branch store has already pruned under its own operator-selected
//! [`BranchRetentionPolicy`] must not stay readable as gameplay detail here. Otherwise an operator
//! who deliberately pruned a branch could still read that branch's observed events through this
//! surface, and the existing policy would be bypassed rather than applied.
//!
//! This module exists so that the two surfaces meet in exactly one place, and so that it applies
//! the decision the branch store already made instead of deriving a second one. The retained store
//! holds no branch eligibility state - it knows no branch age and no branch status - so a second
//! derivation here would be a guess dressed as a policy.
//!
//! What the branch prune selected is disclosed rather than deleted, exactly as this store's own
//! retention discloses it: every observed record keeps its identity and its sequence number and
//! loses its gameplay values, and the window gains a declared span carrying the retention label. A
//! reader therefore sees that a boundary was not retained, rather than reading an empty history as a
//! run that measured nothing.
//!
//! A branch the plan names that this store never captured is not an error: a branch can predate this
//! surface, or can never have carried semantic events. Those branches are counted rather than
//! silently ignored, so an operator can see what the plan named and what this store actually held.

use crate::{BranchPrunePlan, BranchRetentionPolicy};

/// One application of the branch store's existing retention decision to the retained history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticBranchRetentionRequest {
    /// Caller operation identity, so a re-delivered application is recognised rather than repeated.
    pub operation_id: String,
    /// Experiment whose branches the existing plan selected.
    pub experiment_id: String,
    /// The existing policy the branch store selected under, recorded as the operation's provenance.
    pub policy: BranchRetentionPolicy,
    /// The plan the branch store's own prune produced, taken here as authoritative.
    pub plan: BranchPrunePlan,
}

/// What one application of the branch store's retention decision did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticBranchRetentionOutcome {
    /// Branches whose retained history this call disclosed.
    pub disclosed_branches: usize,
    /// Observed records whose gameplay values this call destroyed.
    pub disclosed_records: usize,
    /// Branches the plan named that this store holds no history for.
    pub absent_branches: usize,
    /// Branches whose disclosure this store had already recorded under the same identity.
    pub already_applied: usize,
}
