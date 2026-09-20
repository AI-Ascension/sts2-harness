// SPDX-License-Identifier: MIT

//! The stored event: its origin, the detail it states, and its optional stated causal parent.

use super::{
    Error, SemanticHistoryCoverage, SemanticHistoryKind, SemanticHistoryNamespace,
    SemanticHistoryReference, SemanticHistorySubject, validate_history_identity, validate_label,
};
use serde::{Deserialize, Serialize};

#[path = "semantic_history_model_causal_decode.rs"]
mod causal_decode;

/// Where one event came from, kept distinct from what it says.
///
/// Origin is stated rather than assumed, because native, derived and imported history carry
/// different warrant. A derived event is one the harness computed from its own observed state; an
/// imported event is one restored from saved history through an owned mod port. Neither is promoted
/// to native here.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistoryOrigin {
    /// Observed directly at the host boundary during this capture.
    Native,
    /// Computed by the owner from its own observed state, not observed as an event.
    Derived,
    /// Restored from saved history through an owned mod port.
    Imported,
}

impl SemanticHistoryOrigin {
    /// Every origin, in a stable order.
    pub const ALL: [Self; 3] = [Self::Native, Self::Derived, Self::Imported];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Derived => "derived",
            Self::Imported => "imported",
        }
    }

    /// Returns whether this origin may state an explicit causal parent.
    ///
    /// An imported event's causality was settled when it was captured; re-deriving a parent for it at
    /// import time would be exactly the snapshot-difference inference this boundary refuses.
    #[must_use]
    pub const fn admits_stated_parent(self) -> bool {
        matches!(self, Self::Native | Self::Derived)
    }
}

/// A value an event states, kept distinct from a value it does not.
/// An unavailable value is a separate variant rather than a zero or an empty string, so an unknown
/// quantity can never be read as a real measurement of zero.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SemanticHistoryValue {
    /// A known bounded integer quantity with its unit.
    Quantity {
        /// The quantity.
        amount: i64,
        /// The owner-defined unit, never inferred.
        unit: String,
    },
    /// A known opaque identity reference.
    Reference {
        /// Which namespace the referenced identity belongs to.
        namespace: SemanticHistoryNamespace,
        /// The opaque identity.
        identity: String,
    },
    /// A known owner-defined label.
    Label {
        /// The label text.
        text: String,
    },
    /// The boundary cannot state this value, and says so rather than inventing one.
    Unavailable {
        /// Why the value is unavailable.
        reason: String,
    },
}

impl SemanticHistoryValue {
    /// Validates the value, including that no arm can stand in for another.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Quantity { unit, .. } => validate_label(unit, "value.unit"),
            Self::Reference {
                namespace,
                identity,
            } => {
                validate_history_identity(identity, "value.identity")?;
                if *namespace == SemanticHistoryNamespace::LiveInstance {
                    // A live instance is only valid inside one run and epoch; a value that outlives
                    // its run would be a dangling reference, so it is not admitted here.
                    return Err(Error::InvalidField("value.namespace"));
                }
                Ok(())
            }
            Self::Label { text } => validate_label(text, "value.text"),
            Self::Unavailable { reason } => validate_label(reason, "value.reason"),
        }
    }

    /// Returns whether this value is a real observation.
    #[must_use]
    pub const fn is_stated(&self) -> bool {
        !matches!(self, Self::Unavailable { .. })
    }
}

/// One event as supplied by the owner for storage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryEventInput {
    /// Event identity, opaque and unique inside its run and branch.
    pub event_id: String,
    /// What happened.
    pub kind: SemanticHistoryKind,
    /// Host sequence number, monotonic inside one run, branch, episode and epoch.
    pub sequence: u64,
    /// Episode number the event belongs to.
    pub episode: u64,
    /// Authority epoch the event belongs to.
    pub authority_epoch: u64,
    /// Where the event came from.
    pub origin: SemanticHistoryOrigin,
    /// Which ends of the event the host named, at most one entry per role.
    pub subjects: Vec<SemanticHistorySubject>,
    /// The value the event states, if its kind states one.
    pub value: Option<SemanticHistoryValue>,
    /// The content the event is about, if its kind names content.
    pub reference: Option<SemanticHistoryReference>,
    /// This event's own coverage.
    pub coverage: SemanticHistoryCoverage,
}

/// One stored event, with the order and provenance this boundary assigned.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryEvent {
    /// The schema this record was written under.
    pub schema: String,
    /// The input the owner supplied.
    pub input: SemanticHistoryEventInput,
    /// The branch this event was appended to.
    pub branch_id: String,
    /// The causal parent, stated or explicitly absent.
    pub causal_parent: SemanticHistoryCausalParent,
    /// Digest of the content, used to detect a conflicting re-append.
    pub content_digest: String,
}

impl SemanticHistoryEvent {
    /// The stable identity of this event inside one run and branch.
    #[must_use]
    pub fn event_key(&self) -> (&str, &str) {
        (&self.branch_id, &self.input.event_id)
    }
}

/// A causal parent that is either explicitly stated or explicitly absent.
///
/// The two-arm shape is the contract: a stated parent must name an event, and a not-stated parent
/// must name nothing. A pairing that mixes the arms is refused rather than read as one of them, so
/// "we do not know why" can never be mistaken for "this was the cause".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SemanticHistoryCausalParent {
    /// The host stated which event caused this one.
    Stated {
        /// Identity of the causing event, in the same branch and epoch.
        event_id: String,
    },
    /// The host did not state a cause, and this boundary does not invent one.
    NotStated,
}

impl SemanticHistoryCausalParent {
    /// Validates the arm's own shape, independent of the store's contents.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Stated { event_id } => {
                validate_history_identity(event_id, "causal_parent.event_id")
            }
            Self::NotStated => Ok(()),
        }
    }

    /// Returns the stated parent identity, if one was stated.
    #[must_use]
    pub fn stated_event_id(&self) -> Option<&str> {
        match self {
            Self::Stated { event_id } => Some(event_id),
            Self::NotStated => None,
        }
    }

    /// Returns whether a parent was stated.
    #[must_use]
    pub const fn is_stated(&self) -> bool {
        matches!(self, Self::Stated { .. })
    }
}
