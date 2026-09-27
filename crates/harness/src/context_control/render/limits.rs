// SPDX-License-Identifier: MIT

//! The renderer's advertised budget and its refusal vocabulary.
//!
//! `ContextRenderLimits` carries the values the authoritative owner advertises in its binding's
//! `ContextEffectiveLimits`; the harness maxima stay the outer bound, and enforcing them is what
//! the renderer has always done, so passing the maxima preserves existing behaviour exactly.
//! `ContextRenderError` is the complete set of ways a render can be refused. Both are separated
//! from the request assembly so the budget the renderer advertises can be read, and changed, as
//! one unit.

use super::super::types::{
    MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES, MAX_OBJECTIVE_BYTES,
};

/// Selected owner/profile limits a managed render must respect.
///
/// These are the values the authoritative owner advertises in its binding's
/// `ContextEffectiveLimits`. The harness maxima stay the outer bound; enforcing them is what the
/// renderer has always done, so passing the maxima here preserves existing behaviour exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextRenderLimits {
    pub max_items: usize,
    pub max_notes: usize,
    pub max_context_bytes: usize,
    pub max_objective_bytes: usize,
    /// Output capacity reserved beside `max_context_bytes`, or `None` when this owner publishes no
    /// separate reserve.
    ///
    /// `None` is the pre-existing contract: `max_context_bytes` bounds the input bytes alone and
    /// response capacity stays bounded independently by the provider configuration. When a reserve
    /// is published, `max_context_bytes` is the combined whole-input bound and the served decision
    /// admits `input + reserve` against it, so the reserve is never silently folded into the input.
    pub output_reserve_bytes: Option<usize>,
}

impl ContextRenderLimits {
    #[must_use]
    pub fn harness_maxima() -> Self {
        Self {
            max_items: MAX_CONTEXT_ITEMS,
            max_notes: MAX_CONTEXT_NOTES,
            max_context_bytes: MAX_CONTEXT_BYTES,
            max_objective_bytes: MAX_OBJECTIVE_BYTES,
            output_reserve_bytes: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextRenderError {
    InvalidInput(&'static str),
    UnknownItem,
    ProtectedItem,
    ExpiredItem,
    InvalidUtf8,
    TooLarge,
    ExceedsSelectedLimit(&'static str),
    Encode,
}

impl std::fmt::Display for ContextRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput(message) => message,
            Self::UnknownItem => "selected item is unavailable",
            Self::ProtectedItem => "protected item cannot be selected for editing",
            Self::ExpiredItem => "selected item is expired",
            Self::InvalidUtf8 => "selected item is not valid UTF-8",
            Self::TooLarge => "prepared context exceeds its bound",
            Self::ExceedsSelectedLimit(_) => "prepared context exceeds the selected owner limit",
            Self::Encode => "prepared context encoding failed",
        })
    }
}

impl std::error::Error for ContextRenderError {}
