// SPDX-License-Identifier: MIT

//! Capture coverage: what was observed, what was dropped, and what cannot be stated.

use super::{Error, MAX_HISTORY_LABEL_BYTES, validate_label};
use serde::{Deserialize, Serialize};

/// The coverage state of one event or one interval of a history.
///
/// Coverage belongs to the record, not to the reader. A `Dropped` or `Unsupported` interval is
/// disclosed so a consumer can see that this history is incomplete; neither is ever closed by an
/// invented event, a zeroed quantity or a renumbered sequence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistoryCoverageStatus {
    /// Observed at the host boundary.
    Captured,
    /// Not observed because capture dropped it.
    Dropped,
    /// Not expressible in this vocabulary, so it is not represented as an event.
    Unsupported,
}

impl SemanticHistoryCoverageStatus {
    /// Every status, in a stable order.
    pub const ALL: [Self; 3] = [Self::Captured, Self::Dropped, Self::Unsupported];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Dropped => "dropped",
            Self::Unsupported => "unsupported",
        }
    }

    /// Returns whether an event with this status carries authoritative gameplay values.
    #[must_use]
    pub const fn is_observed(self) -> bool {
        matches!(self, Self::Captured)
    }

    /// Returns whether this status denotes a gap rather than an observation.
    #[must_use]
    pub const fn is_gap(self) -> bool {
        !self.is_observed()
    }
}

/// What one recorded event can say about itself.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryCoverage {
    /// Whether this event was observed, dropped or is unsupported here.
    pub status: SemanticHistoryCoverageStatus,
    /// Optional bounded owner-defined label naming the reason, never a substitute for a value.
    pub label: Option<String>,
}

impl SemanticHistoryCoverage {
    /// A captured event with no extra label.
    #[must_use]
    pub const fn captured() -> Self {
        Self {
            status: SemanticHistoryCoverageStatus::Captured,
            label: None,
        }
    }

    /// A disclosed gap carrying a bounded owner-defined reason.
    #[must_use]
    pub fn gap(status: SemanticHistoryCoverageStatus, label: &str) -> Self {
        Self {
            status,
            label: Some(label.to_owned()),
        }
    }

    /// Validates this coverage record.
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(label) = &self.label {
            validate_label(label, "coverage.label")?;
        }
        if self.status.is_observed() && self.label.is_some() {
            // A captured event carries no gap reason; admitting one would let a label stand in for
            // a value the boundary did not observe.
            return Err(Error::Coverage);
        }
        Ok(())
    }
}

/// One contiguous span of the sequence whose events are not all captured.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryCoverageInterval {
    /// First sequence number in the span.
    pub from_sequence: u64,
    /// Last sequence number in the span, inclusive.
    pub to_sequence: u64,
    /// Why this span is not fully captured.
    pub status: SemanticHistoryCoverageStatus,
    /// Bounded owner-defined reason.
    pub label: String,
}

impl SemanticHistoryCoverageInterval {
    /// Validates that the span is well formed and denotes a real gap.
    pub fn validate(&self) -> Result<(), Error> {
        if self.status.is_observed() || self.from_sequence > self.to_sequence {
            return Err(Error::Coverage);
        }
        validate_label(&self.label, "coverage_interval.label")?;
        if self.label.len() > MAX_HISTORY_LABEL_BYTES {
            return Err(Error::Coverage);
        }
        Ok(())
    }

    /// Returns whether this span contains the given sequence number.
    #[must_use]
    pub const fn contains(&self, sequence: u64) -> bool {
        sequence >= self.from_sequence && sequence <= self.to_sequence
    }
}

/// Where capture began and which intervals it could not fully observe.
///
/// The window is stated by the owner, never inferred. History before `capture_start` is explicitly
/// outside this capture rather than silently absent, which is what lets a reader answer "unknown
/// before capture" honestly instead of reporting an empty result as if nothing had happened.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryCaptureWindow {
    /// First host sequence number this capture could observe.
    pub capture_start: u64,
    /// How many events the host reported before capture began, if it can say.
    pub history_before_capture: Option<u64>,
    /// Spans inside the window that are not fully captured.
    pub intervals: Vec<SemanticHistoryCoverageInterval>,
}

impl SemanticHistoryCaptureWindow {
    /// A window with no declared gaps.
    #[must_use]
    pub fn complete(capture_start: u64) -> Self {
        Self {
            capture_start,
            history_before_capture: None,
            intervals: Vec::new(),
        }
    }

    /// Validates the window, including that no two spans overlap.
    pub fn validate(&self) -> Result<(), Error> {
        let mut previous: Option<u64> = None;
        for interval in &self.intervals {
            interval.validate()?;
            if interval.from_sequence < self.capture_start {
                // A gap before capture began is not a gap in this capture; it is unknown history.
                return Err(Error::Coverage);
            }
            if previous.is_some_and(|end| interval.from_sequence <= end) {
                return Err(Error::Coverage);
            }
            previous = Some(interval.to_sequence);
        }
        Ok(())
    }

    /// Returns the declared gap containing the sequence number, if any.
    #[must_use]
    pub fn gap_at(&self, sequence: u64) -> Option<&SemanticHistoryCoverageInterval> {
        self.intervals.iter().find(|item| item.contains(sequence))
    }

    /// Returns whether the sequence number is before capture began.
    #[must_use]
    pub const fn is_before_capture(&self, sequence: u64) -> bool {
        sequence < self.capture_start
    }
}
