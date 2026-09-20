// SPDX-License-Identifier: MIT

//! The branch store's retention decision applied over the retained store.
//!
//! This sits under the store module for the same reason the prune operations do: it rewrites a
//! retained history's records, capture window and disclosure as one commit, and keeping it here
//! leaves the retained state private to the module that owns it.

use std::collections::BTreeSet;

use super::super::branch_retention::{
    SemanticBranchRetentionOutcome, SemanticBranchRetentionRequest,
};
use super::super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::super::ingest::validate_identity;
use super::super::retention::{
    SemanticRetentionPolicy, apply_prune, intervals_covering, merge_intervals, plan_prune,
};
use super::{SemanticHistoryStore, file};

/// The selection an operator's branch prune implies: once the branch is pruned, nothing it holds is
/// protected any more, so every observed record is disclosed rather than only the ones a history's
/// own protective policy would have selected.
const BRANCH_PRUNED: SemanticRetentionPolicy = SemanticRetentionPolicy {
    retain_observed: false,
    retain_latest: 0,
};

impl SemanticHistoryStore {
    /// Applies the branch store's own retention decision to the retained histories it selected.
    ///
    /// The plan is taken as authoritative rather than recomputed: the branch store owns branch age
    /// and status, and this store owns none of that, so a second derivation here would be a guess.
    /// Every branch the plan names that this store retains has its observed detail disclosed, and a
    /// branch it never captured is counted rather than refused, because a branch can predate this
    /// surface. The application is idempotent by operation identity: a re-delivery discloses nothing
    /// a second time, and reusing that identity under another plan is refused.
    pub fn apply_branch_retention(
        &mut self,
        request: &SemanticBranchRetentionRequest,
    ) -> SemanticHistoryResult<SemanticBranchRetentionOutcome> {
        validate_identity(&request.operation_id)?;
        validate_identity(&request.experiment_id)?;
        if request.plan.branch_ids.is_empty() {
            return Err(SemanticHistoryError::new(Refusal::NothingPrunable));
        }
        let payload = file::digest_branch_retention(request)?;
        let mut pending = Vec::new();
        let mut absent_branches = 0;
        let mut already_applied = 0;
        for branch_id in &request.plan.branch_ids {
            validate_identity(branch_id)?;
            let Some(history) = self.state.histories.get(branch_id) else {
                absent_branches += 1;
                continue;
            };
            if let Some(previous) = history.operation_ids.get(&request.operation_id) {
                if *previous != payload {
                    return Err(SemanticHistoryError::about(
                        Refusal::IdempotencyConflict,
                        &request.operation_id,
                    ));
                }
                already_applied += 1;
                continue;
            }
            pending.push(branch_id.clone());
        }
        let mut disclosed_branches = 0;
        let mut disclosed_records = 0;
        for branch_id in pending {
            let disclosed = self.disclose_branch(&branch_id, &request.operation_id, &payload)?;
            disclosed_branches += 1;
            disclosed_records += disclosed;
        }
        Ok(SemanticBranchRetentionOutcome {
            disclosed_branches,
            disclosed_records,
            absent_branches,
            already_applied,
        })
    }

    /// Discloses every observed value one pruned branch held, as a single commit.
    ///
    /// A branch that holds no observed detail still records the operation identity, so a
    /// re-delivery of the same plan is recognised instead of being re-examined.
    fn disclose_branch(
        &mut self,
        branch_id: &str,
        operation_id: &str,
        payload: &str,
    ) -> SemanticHistoryResult<usize> {
        let history = self
            .state
            .histories
            .get(branch_id)
            .ok_or_else(|| SemanticHistoryError::new(Refusal::UnknownBranch))?;
        let plan = plan_prune(branch_id, &history.records, &BRANCH_PRUNED)?;
        let empty = plan.is_empty();
        let (records, window) = if empty {
            (history.records.clone(), history.window.clone())
        } else {
            apply_prune(&history.records, &history.window, &plan)?
        };
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
            .remove(branch_id)
            .ok_or_else(|| SemanticHistoryError::new(Refusal::UnknownBranch))?;
        history.records = records;
        history.window = window;
        history.retention_intervals = retention_intervals;
        history
            .operation_ids
            .insert(operation_id.to_owned(), payload.to_owned());
        self.commit(branch_id.to_owned(), history)?;
        Ok(pruned.len())
    }
}
