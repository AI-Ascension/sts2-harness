// SPDX-License-Identifier: MIT

//! Backfill over the retained store: the one write path saved history enters through.
//!
//! This sits under the store module beside the prune operations for the same reason: it is a caller
//! that rewrites a retained history's records and capture window as a single commit, and keeping it
//! here leaves the retained state private to the module that owns it. Restored history predates the
//! capture, so it lands ahead of the retained start rather than being appended to it.

use super::super::backfill::SemanticBackfillRequest;
use super::super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::super::ingest::admit_batch;
use super::super::record::SemanticEventBatch;
use super::super::retention::merge_intervals;
use super::super::vocabulary::SemanticEventOrigin;
use super::{SemanticHistoryStore, file};

impl SemanticHistoryStore {
    /// Restores one span of saved history ahead of the retained capture, once per backfill identity.
    ///
    /// Saved history predates anything this harness watched, so it is never admitted as capture: it
    /// lands **before** the retained capture start, and every record must arrive labelled `Imported`
    /// with the coverage label its own source stated. A restored record that claims a native origin,
    /// or a gap that lost its label, is refused rather than relabelled, and a span that does not
    /// abut the retained start is refused rather than leaving an undeclared hole behind it. Every
    /// check runs before the retained state is touched, so a refused restore changes nothing.
    pub(in super::super) fn restore(
        &mut self,
        request: &SemanticBackfillRequest,
        batch: &SemanticEventBatch,
    ) -> SemanticHistoryResult<usize> {
        let payload = file::digest_backfill(request)?;
        let branch_id = &request.fence.branch_id;
        let history = self
            .state
            .histories
            .get(branch_id)
            .ok_or_else(|| SemanticHistoryError::about(Refusal::UnknownBranch, branch_id))?;
        if let Some(previous) = history.operation_ids.get(&request.operation_id) {
            return if *previous == payload {
                Ok(0)
            } else {
                Err(SemanticHistoryError::about(
                    Refusal::IdempotencyConflict,
                    &request.operation_id,
                ))
            };
        }
        if history.binding != request.binding {
            return Err(SemanticHistoryError::about(
                Refusal::BindingMismatch,
                branch_id,
            ));
        }
        if !request.fence.admits(&history.scope) {
            return Err(SemanticHistoryError::about(Refusal::StaleFence, branch_id));
        }
        if batch.scope != history.scope {
            return Err(SemanticHistoryError::new(Refusal::ScopeMismatch));
        }
        let records = admit_batch(&request.binding, batch)?;
        if request.last_sequence < request.first_sequence
            || batch.window.capture_start_sequence != request.first_sequence
            || records.len() as u64 != request.last_sequence - request.first_sequence + 1
        {
            return Err(SemanticHistoryError::about(
                Refusal::BackfillSpan,
                branch_id,
            ));
        }
        for record in &records {
            if record.is_observed() {
                if record.event.origin != Some(SemanticEventOrigin::Imported) {
                    return Err(SemanticHistoryError::about(
                        Refusal::BackfillLabel,
                        record.event_id(),
                    ));
                }
            } else if record
                .event
                .coverage
                .label
                .as_deref()
                .is_none_or(str::is_empty)
            {
                return Err(SemanticHistoryError::about(
                    Refusal::BackfillLabel,
                    record.event_id(),
                ));
            }
        }
        if request.last_sequence.saturating_add(1) != history.window.capture_start_sequence {
            return Err(SemanticHistoryError::about(
                Refusal::BackfillSpan,
                branch_id,
            ));
        }
        let restored = records.len();
        let window = super::super::record::SemanticCaptureWindow {
            capture_start_sequence: request.first_sequence,
            history_before_capture: request.first_sequence != 1,
            intervals: merge_intervals(&batch.window.intervals, &history.window.intervals)?,
        };
        super::super::window::validate_window(&window)?;
        let mut history = self
            .state
            .histories
            .remove(branch_id)
            .ok_or_else(|| SemanticHistoryError::new(Refusal::UnknownBranch))?;
        let mut merged = records;
        merged.extend(history.records);
        history.records = merged;
        history.window = window;
        history
            .operation_ids
            .insert(request.operation_id.clone(), payload);
        self.commit(branch_id.clone(), history)?;
        Ok(restored)
    }
}
