// SPDX-License-Identifier: MIT

//! Typed attachment to the harness-owned context-control authority.
//!
//! This module is deliberately a management port. It carries only immutable,
//! bounded identities and redacted capability metadata; context bytes, provider
//! credentials, game state and transports remain behind their owning ports.

use serde::{Deserialize, Serialize};

use super::auth::AuthContext;
use super::contract::{RunSnapshot, validate_digest, validate_identifier};
use super::service::ManagementError;
use crate::context_control::{
    ContextBoundary, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES, MAX_OBJECTIVE_BYTES,
};
use crate::sha256_hex;

pub const CONTEXT_OWNER_BINDING_SCHEMA_VERSION: &str = "ascension.context-control.owner-binding.v1";
pub const CONTEXT_OWNER_CATALOG_SCHEMA_VERSION: &str = "ascension.context-control.owner-catalog.v1";
pub const CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION: &str = "ascension.context-control.owner-receipt.v1";
pub const MAX_CONTEXT_BINDINGS: usize = 128;
pub const MAX_CONTEXT_SOURCES: usize = 16;
pub const MAX_CONTEXT_OPERATIONS: usize = 16;
pub const MAX_CONTEXT_NODE_KINDS: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextBindingState {
    Available,
    Disabled,
    Unattached,
    Denied,
    Stale,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextBindingOperation {
    IncludeItem,
    ExcludeItem,
    PinItem,
    UnpinItem,
    PutNote,
    RemoveNote,
    SetObjective,
    RestoreConfiguration,
    Pause,
    Commit,
    Resume,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingSource {
    pub source_id: String,
    pub version: u64,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextEffectiveLimits {
    pub max_items: u64,
    pub max_notes: u64,
    pub max_context_bytes: u64,
    pub max_objective_bytes: u64,
    pub max_control_events: u64,
}

impl Default for ContextEffectiveLimits {
    fn default() -> Self {
        Self {
            max_items: MAX_CONTEXT_ITEMS as u64,
            max_notes: MAX_CONTEXT_NOTES as u64,
            max_context_bytes: MAX_CONTEXT_BYTES as u64,
            max_objective_bytes: MAX_OBJECTIVE_BYTES as u64,
            max_control_events: 4096,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingContinuity {
    pub survives_controller_restart: bool,
    pub receipt_recovery: bool,
    pub provider_session_continuity: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingGrants {
    pub metadata_read: bool,
    pub content_read: bool,
    pub edit: bool,
    pub control: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingDescriptor {
    pub schema_version: String,
    pub binding_id: String,
    pub version: u64,
    pub digest: String,
    pub context_ref: String,
    pub node_kinds: Vec<String>,
    pub sources: Vec<ContextBindingSource>,
    pub operations: Vec<ContextBindingOperation>,
    pub effective_limits: ContextEffectiveLimits,
    pub continuity: ContextBindingContinuity,
    pub grants: ContextBindingGrants,
    pub state: ContextBindingState,
}

impl ContextBindingDescriptor {
    pub fn seal(mut self) -> Result<Self, ManagementError> {
        self.digest.clear();
        let bytes = serde_json::to_vec(&self).map_err(|error| {
            ManagementError::invalid("context_binding_encode", error.to_string())
        })?;
        self.digest = sha256_hex(bytes);
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_BINDING_SCHEMA_VERSION
            || self.version == 0
            || self.node_kinds.is_empty()
            || self.node_kinds.len() > MAX_CONTEXT_NODE_KINDS
            || self.sources.len() > MAX_CONTEXT_SOURCES
            || self.operations.len() > MAX_CONTEXT_OPERATIONS
        {
            return Err(ManagementError::invalid(
                "context_binding_descriptor_invalid",
                "context binding descriptor is outside its bounds",
            ));
        }
        for (field, value) in [
            ("context_binding_id", self.binding_id.as_str()),
            ("context_ref", self.context_ref.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        validate_digest("context_binding_digest", &self.digest)?;
        let mut seen = std::collections::BTreeSet::new();
        for kind in &self.node_kinds {
            validate_identifier("context_binding_node_kind", kind)?;
            if !seen.insert(kind) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_node_kind",
                    "context binding node kinds must be unique",
                ));
            }
        }
        let mut seen_sources = std::collections::BTreeSet::new();
        for source in &self.sources {
            validate_identifier("context_binding_source_id", &source.source_id)?;
            if source.version == 0 {
                return Err(ManagementError::invalid(
                    "context_binding_source_version",
                    "context binding source version must be positive",
                ));
            }
            validate_digest("context_binding_source_digest", &source.digest)?;
            if !seen_sources.insert((&source.source_id, source.version)) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_source",
                    "context binding sources must be unique",
                ));
            }
        }
        let mut seen_operations = std::collections::BTreeSet::new();
        for operation in &self.operations {
            if !seen_operations.insert(operation) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_operation",
                    "context binding operations must be unique",
                ));
            }
        }
        validate_limits(&self.effective_limits)?;
        validate_grants(&self.grants)?;
        if matches!(self.state, ContextBindingState::Available)
            && self.operations.is_empty()
            && self.grants.control
        {
            return Err(ManagementError::invalid(
                "context_binding_control_without_operations",
                "a controllable binding must advertise at least one operation",
            ));
        }
        // Only hash after every nested field has been validated and bounded.
        let expected = self.clone().seal()?.digest;
        if expected != self.digest {
            return Err(ManagementError::conflict(
                "context_binding_digest_mismatch",
                "context binding descriptor digest does not match its immutable fields",
            ));
        }
        Ok(())
    }

    /// Returns whether this immutable descriptor can be used by the named
    /// workflow node kind. A disabled or stale descriptor is never a live
    /// binding, even when its metadata remains discoverable.
    pub fn supports(&self, context_ref: &str, node_kind: &str) -> bool {
        self.context_ref == context_ref
            && self.node_kinds.iter().any(|kind| kind == node_kind)
            && matches!(self.state, ContextBindingState::Available)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingCatalog {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub descriptors: Vec<ContextBindingDescriptor>,
}

impl ContextBindingCatalog {
    pub fn validate(&self) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_CATALOG_SCHEMA_VERSION
            || self.descriptors.len() > MAX_CONTEXT_BINDINGS
        {
            return Err(ManagementError::invalid(
                "context_binding_catalog_invalid",
                "context binding catalog is outside its bounds",
            ));
        }
        validate_identifier("context_owner_id", &self.owner_id)?;
        validate_identifier("context_owner_version", &self.owner_version)?;
        validate_digest("context_catalog_digest", &self.catalog_digest)?;
        let mut identities = std::collections::BTreeSet::new();
        for descriptor in &self.descriptors {
            descriptor.validate()?;
            if !identities.insert((&descriptor.binding_id, descriptor.version)) {
                return Err(ManagementError::conflict(
                    "context_binding_duplicate",
                    "context binding IDs and versions must be unique",
                ));
            }
        }
        let expected = catalog_digest(&self.owner_id, &self.owner_version, &self.descriptors)?;
        if expected != self.catalog_digest {
            return Err(ManagementError::conflict(
                "context_catalog_digest_mismatch",
                "context binding catalog digest does not match its descriptors",
            ));
        }
        Ok(())
    }

    pub fn descriptor_for(
        &self,
        context_ref: &str,
        node_kind: &str,
    ) -> Result<&ContextBindingDescriptor, ManagementError> {
        validate_identifier("context_ref", context_ref)?;
        validate_identifier("context_node_kind", node_kind)?;
        let mut matches = self
            .descriptors
            .iter()
            .filter(|descriptor| descriptor.supports(context_ref, node_kind));
        let Some(descriptor) = matches.next() else {
            return Err(ManagementError::capability(
                "context_binding_unsupported",
                "context owner catalog does not advertise a usable binding for this node",
            ));
        };
        if matches.next().is_some() {
            return Err(ManagementError::conflict(
                "context_binding_ambiguous",
                "context owner catalog advertises multiple bindings for this node",
            ));
        }
        Ok(descriptor)
    }
}

#[path = "context_owner_binding.rs"]
mod binding;
#[path = "context_owner_receipt.rs"]
mod receipt;
#[path = "context_owner_support.rs"]
mod support;

pub use binding::{ContextBindingRequest, ContextOwnerBinding};
pub use receipt::{ContextControlCommand, ContextControlCommandKind, ContextControlReceipt};
pub use support::{ContextOwnerPort, UnavailableContextOwnerPort};
pub(crate) use support::{catalog_digest, validate_boundary, validate_grants, validate_limits};
