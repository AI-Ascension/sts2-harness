// SPDX-License-Identifier: MIT

//! Refusals. Every one names the fact that was wrong, never a generic failure.

/// Why one batch, query or traversal was refused.
///
/// Refusal is the module's contract rather than reconstruction: a history that guessed would be
/// worse than one that admits it cannot answer. Each variant therefore names the specific
/// inconsistency, so an operator can tell a gap from a corruption from a scope mismatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticHistoryRefusal {
    /// An identity was empty, over its byte bound, or carried a path, control byte or traversal.
    Identity,
    /// A label or unit was over its byte bound.
    Label,
    /// The batch declared more events than one history may retain.
    TooManyEvents,
    /// The batch declared more coverage intervals than one window may state.
    TooManyIntervals,
    /// The batch's aggregate bytes exceed the retained-history bound.
    TooManyBytes,
    /// A page requested more items than one bounded page may return.
    PageTooLarge,
    /// Two records in one batch occupy the same sequence number.
    DuplicateSequence,
    /// Sequence numbers are not contiguous and ascending from the capture start.
    NonMonotonicSequence,
    /// Two records in one batch share an event identity.
    DuplicateEvent,
    /// A captured record omitted its kind or origin, or a gap supplied one.
    CoverageShape,
    /// A captured record omitted a detail its kind requires.
    MissingDetail,
    /// A captured record supplied a detail its kind does not admit.
    UnexpectedDetail,
    /// A kind that requires a target was stated without one.
    MissingTarget,
    /// A subject named an identity from a namespace that cannot act or be acted on.
    SubjectNamespace,
    /// A subject was stated without the actor role, or an actor was stated twice.
    SubjectRole,
    /// A causal parent was stated for a kind that has no cause to state.
    CausalityNotAdmitted,
    /// A causal parent's identity and provenance disagree.
    CausalShape,
    /// An imported event stated a causal parent.
    ImportedCausality,
    /// A stated parent is not present in the same history.
    StatedParentUnknown,
    /// A stated parent does not precede its child in the same scope.
    StatedParentNotBefore,
    /// A captured record sits inside a declared gap.
    CapturedInsideGap,
    /// A declared gap lies outside the captured range.
    GapOutsideCapture,
    /// A declared span overlaps a span this history already declares or retention already replaced.
    OverlappingIntervals,
    /// The capture window contradicts where capture began.
    WindowContradiction,
    /// A batch or a lookup names a different catalog binding than the retained history.
    BindingMismatch,
    /// The batch names a different scope than the history it would extend.
    ScopeMismatch,
    /// A read named another run, branch, episode or epoch than the history holds.
    StaleFence,
    /// An append identity was reused with a different payload.
    IdempotencyConflict,
    /// The append is not contiguous with the retained history.
    NotContiguous,
    /// A fork named a parent branch that is not retained.
    UnknownParentBranch,
    /// A fork named its own branch as its parent.
    SelfParentBranch,
    /// A retention policy selected nothing, so there is no prune to apply.
    NothingPrunable,
    /// A prune or a lookup named a branch this store does not retain.
    UnknownBranch,
    /// A prune plan does not describe the history it was applied to, so it is refused.
    StalePrunePlan,
    /// The traversal exceeded its visit bound or its depth bound.
    TraversalBound,
    /// The traversal revisited an event, so the causal graph is not a tree.
    CausalCycle,
    /// A lookup was served without the harness granting the historical lookup port.
    LookupNotGranted,
    /// A lookup request or result exceeded its byte bound.
    LookupPayloadTooLarge,
    /// A lookup request was not the shape this port serves.
    LookupShape,
    /// Saved history was restored without the harness granting the owned mod port.
    BackfillNotGranted,
    /// A restored record was not labelled with the origin and coverage label its source stated.
    BackfillLabel,
    /// A restored span did not abut the start of the retained capture.
    BackfillSpan,
    /// The retained store could not be read or written.
    Storage,
}

impl SemanticHistoryRefusal {
    /// The stable lowercase name used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Label => "label",
            Self::TooManyEvents => "too_many_events",
            Self::TooManyIntervals => "too_many_intervals",
            Self::TooManyBytes => "too_many_bytes",
            Self::PageTooLarge => "page_too_large",
            Self::DuplicateSequence => "duplicate_sequence",
            Self::NonMonotonicSequence => "non_monotonic_sequence",
            Self::DuplicateEvent => "duplicate_event",
            Self::CoverageShape => "coverage_shape",
            Self::MissingDetail => "missing_detail",
            Self::UnexpectedDetail => "unexpected_detail",
            Self::MissingTarget => "missing_target",
            Self::SubjectNamespace => "subject_namespace",
            Self::SubjectRole => "subject_role",
            Self::CausalityNotAdmitted => "causality_not_admitted",
            Self::CausalShape => "causal_shape",
            Self::ImportedCausality => "imported_causality",
            Self::StatedParentUnknown => "stated_parent_unknown",
            Self::StatedParentNotBefore => "stated_parent_not_before",
            Self::CapturedInsideGap => "captured_inside_gap",
            Self::GapOutsideCapture => "gap_outside_capture",
            Self::OverlappingIntervals => "overlapping_intervals",
            Self::WindowContradiction => "window_contradiction",
            Self::BindingMismatch => "binding_mismatch",
            Self::ScopeMismatch => "scope_mismatch",
            Self::StaleFence => "stale_fence",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::NotContiguous => "not_contiguous",
            Self::UnknownParentBranch => "unknown_parent_branch",
            Self::SelfParentBranch => "self_parent_branch",
            Self::NothingPrunable => "nothing_prunable",
            Self::UnknownBranch => "unknown_branch",
            Self::StalePrunePlan => "stale_prune_plan",
            Self::TraversalBound => "traversal_bound",
            Self::CausalCycle => "causal_cycle",
            Self::LookupNotGranted => "lookup_not_granted",
            Self::LookupPayloadTooLarge => "lookup_payload_too_large",
            Self::LookupShape => "lookup_shape",
            Self::BackfillNotGranted => "backfill_not_granted",
            Self::BackfillLabel => "backfill_label",
            Self::BackfillSpan => "backfill_span",
            Self::Storage => "storage",
        }
    }
}

/// One refusal together with the subject that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryError {
    /// What was wrong.
    pub refusal: SemanticHistoryRefusal,
    /// The event identity or query subject the refusal is about, when one is known.
    pub subject: Option<String>,
}

impl SemanticHistoryError {
    /// Builds a refusal with no named subject.
    #[must_use]
    pub const fn new(refusal: SemanticHistoryRefusal) -> Self {
        Self {
            refusal,
            subject: None,
        }
    }

    /// Builds a refusal naming the subject it is about.
    #[must_use]
    pub fn about(refusal: SemanticHistoryRefusal, subject: &str) -> Self {
        Self {
            refusal,
            subject: Some(subject.to_owned()),
        }
    }
}

impl std::fmt::Display for SemanticHistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.subject {
            Some(subject) => write!(formatter, "{}: {subject}", self.refusal.name()),
            None => write!(formatter, "{}", self.refusal.name()),
        }
    }
}

impl std::error::Error for SemanticHistoryError {}

/// Convenience result type for every operation in this module.
pub type SemanticHistoryResult<T> = Result<T, SemanticHistoryError>;
