// SPDX-License-Identifier: MIT

//! Opaque identity acceptance and the namespaces history keeps distinct.

use super::{Error, MAX_HISTORY_IDENTITY_BYTES, MAX_HISTORY_IDENTITY_SEGMENTS};
use serde::{Deserialize, Serialize};

/// Returns whether one identity is an opaque token this boundary will carry.
///
/// An identity is opaque: non-empty, bounded, free of control bytes and path separators, and free of
/// the traversal segments that would make it addressable on a host filesystem. An event identity
/// that could be read as a path is refused so a history record can never be turned into a host read.
#[must_use]
pub fn is_opaque_history_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_HISTORY_IDENTITY_BYTES
        && !value.contains(|character: char| character.is_control())
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.contains("..")
        && !value.to_ascii_lowercase().starts_with("file")
        && value.split('.').count() <= MAX_HISTORY_IDENTITY_SEGMENTS
}

/// Validates one opaque identity, naming the field in the refusal.
pub fn validate_history_identity(value: &str, field: &'static str) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_HISTORY_IDENTITY_BYTES {
        return Err(Error::InvalidIdentity(field));
    }
    if is_opaque_history_identity(value) {
        Ok(())
    } else {
        Err(Error::NonOpaqueIdentity(field))
    }
}

/// The identity namespaces a history record may name.
///
/// Definition, live-instance, action and event identities are distinct kinds of thing. Keeping them
/// separate is what stops an event from being read as naming a definition, or an action identity from
/// being used where a live instance is required.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistoryNamespace {
    /// A content definition identity, stable across runs.
    Definition,
    /// A live instance identity, valid only inside one run and epoch.
    LiveInstance,
    /// An action identity offered or taken in one episode.
    Action,
    /// An event identity assigned by this boundary.
    Event,
}

impl SemanticHistoryNamespace {
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

    /// Returns whether this namespace may be the subject of a history event.
    ///
    /// Only a live instance may be a subject. A definition is not a thing that acts, and an action is
    /// what the event records rather than what it happened to.
    #[must_use]
    pub const fn admits_subject(self) -> bool {
        matches!(self, Self::LiveInstance)
    }
}
