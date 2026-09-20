// SPDX-License-Identifier: MIT

//! One content identity an event names, kept separate from the live thing it happened to.

use super::{Error, validate_history_identity, validate_label};
use serde::{Deserialize, Serialize};

/// One content identity an event names, resolved against the content manifest.
///
/// A reference is not a subject: a subject is a live instance that acted or was acted on, and a
/// reference is the content the event is about. Keeping them apart is what stops a definition
/// identity from being read as the live thing that acted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryReference {
    /// Manifest entity kind the identity resolves against.
    pub entity_kind: String,
    /// Opaque identity resolved against the content manifest.
    pub namespaced_id: String,
}

impl SemanticHistoryReference {
    /// Validates the entity kind and the referenced identity.
    pub fn validate(&self) -> Result<(), Error> {
        validate_label(&self.entity_kind, "reference.entity_kind")?;
        validate_history_identity(&self.namespaced_id, "reference.namespaced_id")
    }
}
