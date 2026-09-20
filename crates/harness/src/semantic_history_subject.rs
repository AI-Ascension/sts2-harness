// SPDX-License-Identifier: MIT

//! Who an event is about, on both ends of it.

use super::{Error, SemanticHistoryNamespace, validate_history_identity};
use serde::{Deserialize, Serialize};

/// Which end of an event one subject names.
///
/// The role is stated rather than inferred from position, so an actor and a target that happen to
/// share a token are never read as each other.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHistorySubjectRole {
    /// The subject that caused the event.
    Actor,
    /// The subject the event acted on.
    Target,
}

impl SemanticHistorySubjectRole {
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

/// One named end of an event: the subject, its namespace, and its role.
///
/// The namespace travels with the identity so a consumer can never read an actor that happens to
/// share a token with a definition as that definition. A missing target is expressed by the kind
/// refusing the event, not by an empty subject here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistorySubject {
    /// Which end of the event this subject names.
    pub role: SemanticHistorySubjectRole,
    /// Which namespace this identity belongs to.
    pub namespace: SemanticHistoryNamespace,
    /// The opaque identity inside that namespace.
    pub identity: String,
}

impl SemanticHistorySubject {
    /// Validates the namespace and the opaque identity.
    pub fn validate(&self, field: &'static str) -> Result<(), Error> {
        if !self.namespace.admits_subject() {
            // A definition is not a thing that acts, and an action is what an event records rather
            // than what it happened to; only a live instance may be a subject.
            return Err(Error::WrongSubjectNamespace(self.role.name()));
        }
        validate_history_identity(&self.identity, field)
    }
}
