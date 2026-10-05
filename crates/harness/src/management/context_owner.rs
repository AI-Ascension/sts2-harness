// SPDX-License-Identifier: MIT

//! Typed attachment to the harness-owned context-control authority.
//!
//! This module is deliberately a management port. It carries only immutable,
//! bounded identities and redacted capability metadata; context bytes, provider
//! credentials, game state and transports remain behind their owning ports.

use serde::{Deserialize, Serialize};

#[path = "context_owner_source.rs"]
mod source;
pub use source::*;

use super::auth::AuthContext;
use super::contract::{RunSnapshot, validate_digest, validate_identifier};
use super::service::ManagementError;
use crate::context_control::{
    ContextBoundary, ContextRenderLimits, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES,
    MAX_CONTROL_EVENTS, MAX_OBJECTIVE_BYTES,
};
use crate::sha256_hex;

pub const CONTEXT_OWNER_BINDING_SCHEMA_VERSION: &str = "ascension.context-control.owner-binding.v1";
pub const CONTEXT_OWNER_CATALOG_SCHEMA_VERSION: &str = "ascension.context-control.owner-catalog.v1";
/// Receipt v2 binds command and post-transition boundary identity. The v1
/// wire shape omitted those fields, so it cannot be safely upgraded.
pub const CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION: &str = "ascension.context-control.owner-receipt.v2";
pub const CONTEXT_OWNER_RECEIPT_V1_SCHEMA_VERSION: &str =
    "ascension.context-control.owner-receipt.v1";
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
    /// Output capacity this owner reserves beside the whole input, or `None` for no separate
    /// reserve: then `max_context_bytes` bounds the input bytes alone, which is the prior contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_reserve_bytes: Option<u64>,
}

impl Default for ContextEffectiveLimits {
    fn default() -> Self {
        Self {
            max_items: MAX_CONTEXT_ITEMS as u64,
            max_notes: MAX_CONTEXT_NOTES as u64,
            max_context_bytes: MAX_CONTEXT_BYTES as u64,
            max_objective_bytes: MAX_OBJECTIVE_BYTES as u64,
            max_control_events: 4096,
            output_reserve_bytes: None,
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

include!("context_owner_descriptor.rs");
include!("context_owner_catalog.rs");

#[path = "context_owner_binding.rs"]
mod binding;
#[path = "context_owner_composition.rs"]
mod composition;
#[path = "context_owner_drafts.rs"]
mod drafts;
#[path = "context_owner_receipt.rs"]
mod receipt;
pub use composition::{CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA, ContextOwnerControlLimits};
pub use support::{CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA, ContextOwnerAssociationView};

#[path = "context_owner_support.rs"]
mod support;

pub use binding::{ContextBindingRequest, ContextOwnerBinding};
pub use composition::{
    CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA, ContextOwnerEffectiveLimitsView,
    ContextOwnerRenderRequest, compose_context_owner_binding,
};
pub use drafts::*;
pub use receipt::{ContextControlCommand, ContextControlCommandKind, ContextControlReceipt};
pub use support::{ContextControlReceiptRecovery, ContextOwnerPort, UnavailableContextOwnerPort};
pub(crate) use support::{catalog_digest, validate_boundary, validate_grants, validate_limits};
