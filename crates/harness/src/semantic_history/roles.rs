// SPDX-License-Identifier: MIT

//! Identity namespaces, subject roles, causal provenance and coverage state.

use serde::{Deserialize, Serialize};

/// Which namespace an identity belongs to.
///
/// The accepted query contract keeps definition identities, live instance identities and action
/// identities distinct. The history therefore stores the namespace each identity was minted in
/// rather than comparing identities as bare strings: two equal tokens from different namespaces are
/// a collision to refuse, never an aliasing to accept.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticIdentityNamespace {
    /// A static definition identity resolved through the content manifest.
    Definition,
    /// A live identity scoped to one run, branch, episode or epoch.
    LiveInstance,
    /// An action identity from the action vocabulary.
    Action,
    /// An event identity minted by this history.
    Event,
}

impl SemanticIdentityNamespace {
    /// Every namespace, in a stable order.
    pub const ALL: [Self; 4] = [
        Self::Definition,
        Self::LiveInstance,
        Self::Action,
        Self::Event,
    ];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::LiveInstance => "live_instance",
            Self::Action => "action",
            Self::Event => "event",
        }
    }

    /// Resolves a producer name to a namespace.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }

    /// Returns whether a subject of an event may be minted in this namespace.
    ///
    /// An actor or a target of an event is something that exists in the live run, so it belongs to
    /// the live-instance namespace. Accepting a definition, action or event identity here would let
    /// an event name a definition as though it were the thing that acted.
    #[must_use]
    pub const fn admits_subject_role(self) -> bool {
        matches!(self, Self::LiveInstance)
    }
}

/// Which end of an event one subject names.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSubjectRole {
    /// The subject that caused the event.
    Actor,
    /// The subject the event acted on.
    Target,
}

impl SemanticSubjectRole {
    /// Every role, in a stable order.
    pub const ALL: [Self; 2] = [Self::Actor, Self::Target];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Target => "target",
        }
    }
}

/// Whether a recorded event states its causal parent.
///
/// The distinction exists so "no parent" and "not yet known" cannot be confused. A history that
/// inferred a parent by diffing snapshots would manufacture causal claims the host never made.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCausalProvenance {
    /// The host named the parent event explicitly.
    Stated,
    /// No parent was named; this is a disclosure, not an inference.
    NotStated,
}

impl SemanticCausalProvenance {
    /// Every provenance, in a stable order.
    pub const ALL: [Self; 2] = [Self::Stated, Self::NotStated];

    /// The stable lowercase name used in owner-defined text and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stated => "stated",
            Self::NotStated => "not_stated",
        }
    }
}

/// The coverage state of one event or one interval of a history.
///
/// Coverage is a property of the record, not of the reader. A `Dropped` or `Unsupported` interval is
/// disclosed so a consumer can see that this history is incomplete; neither is ever closed by an
/// invented event, a zeroed quantity, or a renumbered sequence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCoverageStatus {
    /// Observed at the host boundary.
    Captured,
    /// Not observed because the capture dropped it.
    Dropped,
    /// Not expressible in this vocabulary, so it is not represented as an event.
    Unsupported,
}

impl SemanticCoverageStatus {
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

    /// Resolves a producer name to a coverage status.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }

    /// Returns whether an event with this status carries authoritative gameplay values.
    #[must_use]
    pub const fn is_observed(self) -> bool {
        matches!(self, Self::Captured)
    }
}
