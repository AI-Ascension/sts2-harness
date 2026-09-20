// SPDX-License-Identifier: MIT

//! Prune operations over the retained store: the preview an operator inspects, the application that
//! replaces a selected span with a declaration, and the disclosure the history keeps afterwards.
//!
//! These operations sit under the store module instead of beside it because they are the only
//! callers that rewrite one retained history's records, capture window and disclosure as a single
//! commit. Keeping them here leaves the retained state private to the module that owns it, so every
//! other module still reaches a pruned history through the store's own reads.

use std::collections::BTreeSet;

use super::super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::super::ingest::validate_identity;
use super::super::record::SemanticCoverageInterval;
use super::super::retention::{
    SemanticPrunePlan, SemanticPruneRequest, apply_prune, intervals_covering, merge_intervals,
    plan_prune,
};
use super::{SemanticHistoryStore, file};

impl SemanticHistoryStore {
    /// Computes what a retention policy would disclose for one branch, changing nothing.
    ///
    /// The preview is the operator's chance to see exactly which events a prune would destroy before
    /// it destroys them, so this is the only way to obtain a plan the store will accept.
    pub fn prune_preview(
        &self,
        request: &SemanticPruneRequest,
    ) -> SemanticHistoryResult<SemanticPrunePlan> {
        validate_identity(&request.branch_id)?;
        let history = self
            .state
            .histories
            .get(&request.branch_id)
            .ok_or_else(|| {
                SemanticHistoryError::about(Refusal::UnknownBranch, &request.branch_id)
            })?;
        plan_prune(&request.branch_id, &history.records, &request.policy)
    }

    /// Discloses the spans a previewed policy selects, once per prune identity.
    ///
    /// The plan must be the one this history produces now: a plan computed against other bytes, or
    /// against a policy that no longer selects the same records, is refused rather than applied to
    /// a history it does not describe. Re-delivering an applied prune returns the same plan, and
    /// reusing its identity with a different policy is a conflict.
    pub fn prune(
        &mut self,
        operation_id: &str,
        request: &SemanticPruneRequest,
        plan: &SemanticPrunePlan,
    ) -> SemanticHistoryResult<SemanticPrunePlan> {
        validate_identity(operation_id)?;
        validate_identity(&request.branch_id)?;
        let payload = file::digest_prune(request)?;
        let history = self
            .state
            .histories
            .get(&request.branch_id)
            .ok_or_else(|| {
                SemanticHistoryError::about(Refusal::UnknownBranch, &request.branch_id)
            })?;
        if let Some(previous) = history.operation_ids.get(operation_id) {
            return if *previous == payload {
                history
                    .prune_plans
                    .get(operation_id)
                    .cloned()
                    .ok_or_else(|| SemanticHistoryError::new(Refusal::Storage))
            } else {
                Err(SemanticHistoryError::about(
                    Refusal::IdempotencyConflict,
                    operation_id,
                ))
            };
        }
        let current = plan_prune(&request.branch_id, &history.records, &request.policy)?;
        if *plan != current {
            return Err(SemanticHistoryError::new(Refusal::StalePrunePlan));
        }
        let (records, window) = apply_prune(&history.records, &history.window, plan)?;
        let pruned = plan
            .prunable_sequences
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let intervals = intervals_covering(&pruned);
        let retention_intervals = merge_intervals(&history.retention_intervals, &intervals)?;
        let mut history = self
            .state
            .histories
            .remove(&request.branch_id)
            .ok_or_else(|| SemanticHistoryError::new(Refusal::UnknownBranch))?;
        history.records = records;
        history.window = window;
        history.retention_intervals = retention_intervals;
        history
            .operation_ids
            .insert(operation_id.to_owned(), payload);
        history
            .prune_plans
            .insert(operation_id.to_owned(), plan.clone());
        self.commit(request.branch_id.clone(), history)?;
        Ok(plan.clone())
    }

    /// Returns the spans retention has replaced with a disclosed gap on one branch.
    #[must_use]
    pub fn retention_intervals(&self, branch_id: &str) -> Option<&[SemanticCoverageInterval]> {
        self.state
            .histories
            .get(branch_id)
            .map(|history| history.retention_intervals.as_slice())
    }
}
