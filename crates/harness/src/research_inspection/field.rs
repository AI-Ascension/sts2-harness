// SPDX-License-Identifier: MIT

//! The supported private-field matrix and its availability vocabulary.
//!
//! A research read names field groups rather than arbitrary paths: there is no
//! object, reflection, memory or file query in this contract. Every group is a
//! bounded, stably-labelled set of field references the native capture owner
//! understands, so a request that names an unknown group is refused instead of
//! being passed through to the native owner as free text.
//!
//! Availability is reported per field and is never collapsed into a value. A
//! field the capture did not materialize is `NotMaterialized`, not zero and not
//! an empty string; a field whose value depends on a future outcome is
//! `SimulationRequired` with its dependency named, and this contract never
//! simulates it.

use serde::{Deserialize, Serialize};

/// Maximum bytes of a research field reference.
pub const MAX_FIELD_REF_BYTES: usize = 96;

/// A bounded, stably-labelled group of private checkpoint fields.
///
/// The groups are the supported matrix: they are the only private data this
/// contract can name. Adding a group is additive and never reinterprets an
/// existing label.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchFieldGroup {
    /// Exact RNG stream identities, cursors and draw counters as captured.
    RngStreams,
    /// Hidden ordered piles: draw order, contents and positions as stored.
    HiddenPiles,
    /// Pre-generated assignments the client has not revealed yet.
    UnrevealedAssignments,
    /// Pending hidden state that a decision has not yet resolved.
    PendingHiddenState,
    /// Save-state scalar fields the capture coverage manifest lists.
    SaveStateFields,
}

impl ResearchFieldGroup {
    /// Returns the stable label of this group.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RngStreams => "rng_streams",
            Self::HiddenPiles => "hidden_piles",
            Self::UnrevealedAssignments => "unrevealed_assignments",
            Self::PendingHiddenState => "pending_hidden_state",
            Self::SaveStateFields => "save_state_fields",
        }
    }

    /// Every supported group, in the stable order a matrix document lists them.
    #[must_use]
    pub const fn supported() -> [Self; 5] {
        [
            Self::RngStreams,
            Self::HiddenPiles,
            Self::UnrevealedAssignments,
            Self::PendingHiddenState,
            Self::SaveStateFields,
        ]
    }
}

/// One bounded private-field reference inside a group.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchFieldRef {
    /// The group this field belongs to.
    pub group: ResearchFieldGroup,
    /// The owner-defined field name inside that group.
    pub field: String,
}

impl ResearchFieldRef {
    /// Builds a field reference, refusing an unusable field name.
    ///
    /// The name is a bounded lowercase identifier: letters, digits, `_` and
    /// `.`. Anything else is refused before it can reach the native owner, so a
    /// path, a query expression or an object reference can never be expressed.
    pub fn new(
        group: ResearchFieldGroup,
        field: impl Into<String>,
    ) -> Result<Self, super::ResearchInspectionError> {
        let field = field.into();
        let usable = !field.is_empty()
            && field.len() <= MAX_FIELD_REF_BYTES
            && !field.starts_with('.')
            && !field.ends_with('.')
            && !field.contains("..")
            && field.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.')
            });
        if !usable {
            return Err(super::ResearchInspectionError::InvalidField);
        }
        Ok(Self { group, field })
    }

    /// The group this field belongs to.
    #[must_use]
    pub const fn group(&self) -> ResearchFieldGroup {
        self.group
    }

    /// The owner-defined field name.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }
}

/// Per-field availability of a private field in the bound capture.
///
/// Reported instead of a synthesized value. `Available` means the verified
/// capture stored the field; the other variants say why a value is not reported,
/// so a consumer can never read absence as zero, empty or complete.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldAvailability {
    /// The capture stored this field and it may be reported.
    Available {
        /// The reported value, as the capture stored it.
        value: String,
    },
    /// The capture did not store this field at this boundary.
    NotMaterialized,
    /// The field exists but its value depends on a future outcome.
    SimulationRequired {
        /// The dependency that would have to be resolved.
        dependency: String,
    },
    /// The native owner does not expose this field at all.
    Unsupported,
}

impl FieldAvailability {
    /// Whether a value is reported.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// The stable availability label, for records and refusal text.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Available { .. } => "available",
            Self::NotMaterialized => "not_materialized",
            Self::SimulationRequired { .. } => "simulation_required",
            Self::Unsupported => "unsupported",
        }
    }

    /// The reported value, when one is available.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Available { value } => Some(value),
            _ => None,
        }
    }
}
