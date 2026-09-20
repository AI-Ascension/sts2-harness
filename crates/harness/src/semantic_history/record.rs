// SPDX-License-Identifier: MIT

//! The event records a history retains, and the coverage that travels with each one.

use serde::{Deserialize, Serialize};

use super::roles::{
    SemanticCausalProvenance, SemanticCoverageStatus, SemanticIdentityNamespace,
    SemanticSubjectRole,
};
use super::scope::{SemanticCatalogBinding, SemanticEventScope};
use super::vocabulary::{SemanticEventKind, SemanticEventOrigin};

/// A bounded gameplay quantity: an amount and the unit it is stated in.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticQuantity {
    /// Signed amount; a resource change may be negative, an applied effect is never zero.
    pub amount: i64,
    /// Opaque unit the amount is stated in, for example `health`, `block` or `energy`.
    pub unit: String,
}

/// One content identity an event names, resolved against the content manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticReference {
    /// Manifest entity kind the identity resolves against.
    pub entity_kind: String,
    /// Namespaced identity resolved against the content manifest.
    pub namespaced_id: String,
}

/// One named end of an event: the subject, its namespace, and its role.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventSubject {
    /// Which end of the event this subject names.
    pub role: SemanticSubjectRole,
    /// Namespace the identity was minted in.
    pub namespace: SemanticIdentityNamespace,
    /// Opaque subject identity.
    pub subject_id: String,
}

/// The causal parent of one event, paired with whether it was stated.
///
/// The two fields must agree: a named parent is `Stated` and an unnamed parent is `NotStated`. Any
/// other pairing is a refusal, so a consumer can rely on `provenance` alone to decide whether the
/// absence of a parent means anything.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCausalParent {
    /// The parent event identity, present only when one was stated.
    pub parent_event_id: Option<String>,
    /// Whether the parent above was stated by the host.
    pub provenance: SemanticCausalProvenance,
}

impl SemanticCausalParent {
    /// The explicit disclosure that no parent was named.
    #[must_use]
    pub const fn not_stated() -> Self {
        Self {
            parent_event_id: None,
            provenance: SemanticCausalProvenance::NotStated,
        }
    }

    /// Returns the stated parent identity, if one was stated.
    #[must_use]
    pub fn stated_parent(&self) -> Option<&str> {
        match self.provenance {
            SemanticCausalProvenance::Stated => self.parent_event_id.as_deref(),
            SemanticCausalProvenance::NotStated => None,
        }
    }
}

/// What one recorded event can say about itself.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventCoverage {
    /// Whether this event was observed, dropped or is unsupported here.
    pub status: SemanticCoverageStatus,
    /// Optional bounded owner-defined label naming the reason, never a substitute for a value.
    pub label: Option<String>,
}

/// One contiguous span of the sequence whose events are not all captured.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCoverageInterval {
    /// Why the span is incomplete; a `Captured` span is never a declared interval.
    pub status: SemanticCoverageStatus,
    /// First sequence number covered by this span.
    pub first_sequence: u64,
    /// Last sequence number covered by this span, inclusive.
    pub last_sequence: u64,
}

/// What a history says about the extent of its own capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCaptureWindow {
    /// Sequence number at which capture began; the earliest event this history can own.
    pub capture_start_sequence: u64,
    /// Whether gameplay history exists before `capture_start_sequence`.
    pub history_before_capture: bool,
    /// Declared spans inside the captured range that are not fully captured.
    pub intervals: Vec<SemanticCoverageInterval>,
}

impl SemanticCaptureWindow {
    /// Returns the declared span covering one sequence, if any.
    #[must_use]
    pub fn interval_covering(&self, sequence: u64) -> Option<&SemanticCoverageInterval> {
        self.intervals.iter().find(|interval| {
            sequence >= interval.first_sequence && sequence <= interval.last_sequence
        })
    }
}

/// One authoritative event, or one gap the boundary discloses instead of inventing an event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventInput {
    /// Opaque event identity, unique inside its history.
    pub event_id: String,
    /// Sequence number this record occupies inside the batch's scope.
    pub sequence: u64,
    /// Whether this record was observed, dropped or is unsupported here.
    pub coverage: SemanticEventCoverage,
    /// Kind of event; present exactly when the record was captured.
    pub kind: Option<SemanticEventKind>,
    /// Where the event came from; present exactly when the record was captured.
    pub origin: Option<SemanticEventOrigin>,
    /// The subjects of the event, actor first; empty for a disclosed gap.
    pub subjects: Vec<SemanticEventSubject>,
    /// The causal parent, stated or explicitly absent; present exactly when the kind admits a cause.
    pub causal_parent: Option<SemanticCausalParent>,
    /// The gameplay quantity this event reports, for a kind that carries one.
    pub value: Option<SemanticQuantity>,
    /// The content identity this event names, for a kind that names one.
    pub reference: Option<SemanticReference>,
    /// Bounded owner-defined display text, never a substitute for a typed value.
    pub label: Option<String>,
}

impl SemanticEventInput {
    /// Returns whether this record was observed rather than disclosed as a gap.
    #[must_use]
    pub fn is_observed(&self) -> bool {
        self.coverage.status.is_observed()
    }

    /// Returns the retained byte weight of this event's unbounded text.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.event_id.len()
            + self.label.as_ref().map_or(0, String::len)
            + self.coverage.label.as_ref().map_or(0, String::len)
            + self
                .subjects
                .iter()
                .map(|subject| subject.subject_id.len())
                .sum::<usize>()
            + self.value.as_ref().map_or(0, |value| value.unit.len() + 8)
            + self.reference.as_ref().map_or(0, |reference| {
                reference.entity_kind.len() + reference.namespaced_id.len()
            })
    }
}

/// One validated event retained by a history, bound to its catalog and its scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventRecord {
    /// Catalog binding this record was validated against.
    pub binding: SemanticCatalogBinding,
    /// The scope this event's sequence number is monotonic inside.
    pub scope: SemanticEventScope,
    /// Validated event value.
    pub event: SemanticEventInput,
}

impl SemanticEventRecord {
    /// Returns the opaque event identity.
    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event.event_id
    }

    /// Returns whether this record was observed rather than disclosed as a gap.
    #[must_use]
    pub fn is_observed(&self) -> bool {
        self.event.is_observed()
    }
}

/// Owner-reported history: one scope, a capture window, and the events or gaps inside it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventBatch {
    /// The run, branch, episode and epoch every sequence number here is monotonic inside.
    pub scope: SemanticEventScope,
    /// What the boundary observed and which spans it could not fully observe.
    pub window: SemanticCaptureWindow,
    /// Records in ascending sequence order, each occupying exactly one sequence number.
    pub events: Vec<SemanticEventInput>,
}
