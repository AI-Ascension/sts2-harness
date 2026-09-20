// SPDX-License-Identifier: MIT

//! Capture-window admission: where capture began, and which spans it could not fully observe.

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::{SemanticCaptureWindow, SemanticEventInput};
use super::scope::SEMANTIC_MAX_INTERVALS;

/// Validates a capture window's own claims.
///
/// A consumer that cannot see the capture boundary cannot tell an absent event from an unwatched
/// one, so the window must be internally consistent before any event is read against it.
pub(super) fn validate_window(window: &SemanticCaptureWindow) -> SemanticHistoryResult<()> {
    if window.intervals.len() > SEMANTIC_MAX_INTERVALS {
        return Err(SemanticHistoryError::new(Refusal::TooManyIntervals));
    }
    if window.capture_start_sequence == 0 {
        return Err(SemanticHistoryError::new(Refusal::WindowContradiction));
    }
    if window.capture_start_sequence == 1 && window.history_before_capture {
        return Err(SemanticHistoryError::new(Refusal::WindowContradiction));
    }
    let mut previous_end: Option<u64> = None;
    for interval in &window.intervals {
        if interval.status.is_observed() {
            return Err(SemanticHistoryError::new(Refusal::GapOutsideCapture));
        }
        if interval.first_sequence > interval.last_sequence
            || interval.first_sequence < window.capture_start_sequence
        {
            return Err(SemanticHistoryError::new(Refusal::GapOutsideCapture));
        }
        if previous_end.is_some_and(|end| interval.first_sequence <= end) {
            return Err(SemanticHistoryError::new(Refusal::GapOutsideCapture));
        }
        previous_end = Some(interval.last_sequence);
    }
    Ok(())
}

/// Returns whether a sequence sits inside a declared gap of this window.
pub(super) fn inside_declared_gap(window: &SemanticCaptureWindow, sequence: u64) -> bool {
    window.interval_covering(sequence).is_some()
}

/// Validates one disclosed gap against the window it claims to sit inside.
///
/// A gap carries no gameplay detail at all: supplying a kind, a subject or a quantity for a record
/// nothing observed is the invented value this vocabulary exists to refuse.
pub(super) fn validate_gap(
    event: &SemanticEventInput,
    window: &SemanticCaptureWindow,
) -> SemanticHistoryResult<()> {
    if event.kind.is_some()
        || event.origin.is_some()
        || !event.subjects.is_empty()
        || event.causal_parent.is_some()
        || event.value.is_some()
        || event.reference.is_some()
    {
        return Err(SemanticHistoryError::about(
            Refusal::CoverageShape,
            &event.event_id,
        ));
    }
    match window.interval_covering(event.sequence) {
        Some(interval) if interval.status == event.coverage.status => Ok(()),
        Some(_) => Err(SemanticHistoryError::about(
            Refusal::CoverageShape,
            &event.event_id,
        )),
        None => Err(SemanticHistoryError::about(
            Refusal::GapOutsideCapture,
            &event.event_id,
        )),
    }
}
