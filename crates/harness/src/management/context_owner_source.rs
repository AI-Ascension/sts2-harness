// SPDX-License-Identifier: MIT

use super::ContextBindingSource;
use crate::context_control::{
    ContextBoundary, ContextMembershipSelector, ContextRenderLimits, ContextSourceDocument,
    MembershipContinuity,
};
use serde::{Deserialize, Serialize};

pub const CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION: &str =
    "ascension.context-owner.context-source-upload.v1";
pub const CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION: &str =
    "ascension.context-owner.context-source-adoption.v1";
pub const CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION: &str =
    "ascension.context-owner.source-status.v1";
pub const MAX_CONTEXT_BINDINGS: usize = 128;
pub const MAX_CONTEXT_SOURCES: usize = 16;
pub const MAX_CONTEXT_OPERATIONS: usize = 16;
pub const MAX_CONTEXT_NODE_KINDS: usize = 16;

/// Authenticated upload payload for one immutable source identity advertised by the owner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextSourceUpload {
    pub schema_version: String,
    pub document: ContextSourceDocument,
}

/// Explicit adoption of an already published owner source as a control revision.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextSourceAdoptionRequest {
    pub schema_version: String,
    pub idempotency_key: String,
    pub expected_control_version: u64,
    pub expected_revision_id: String,
    pub expected_boundary: ContextBoundary,
}

/// Public metadata for an immutable owner source. Content bytes are never returned.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextSourcePublication {
    pub schema_version: String,
    pub source: ContextBindingSource,
}

/// Read-only owner state needed to publish or explicitly adopt a source before
/// the workflow reaches its first context-bound invocation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerSourceStatus {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub instance_id: String,
    pub boundary: ContextBoundary,
    pub active_revision_id: String,
    pub active_source: Option<crate::context_control::ActiveContextSource>,
}

/// Private source material resolved for one actual live provider decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRenderSource {
    pub source_id: String,
    pub source_version: u64,
    pub source_digest: String,
    pub active_revision_id: String,
    pub boundary: ContextBoundary,
    pub limits: ContextRenderLimits,
    pub document: ContextSourceDocument,
    /// The owner's selector for this invocation, if a membership policy is in force.
    ///
    /// The selector is bound to the invocation identity at render time; it is deliberately not a
    /// finished policy, so it can never carry another invocation's identity.
    pub membership: Option<ContextMembershipSelector>,
    /// The continuity the selected binding can actually execute for this invocation.
    pub continuity: MembershipContinuity,
    /// Owner-issued Unix time used only for source expiry validation.
    pub now: u64,
    /// Earliest expiry of any selected source item.
    pub valid_until: u64,
    pub identity: ContextRenderSourceIdentity,
}

/// Non-content fence rechecked before and after provider inference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRenderSourceIdentity {
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub binding_id: String,
    pub binding_version: u64,
    pub binding_digest: String,
    pub invocation_id: String,
    pub instance_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub active_revision_id: String,
    pub source_id: String,
    pub source_version: u64,
    pub source_digest: String,
    /// Digest of the selector in force, so a selector change during inference is fenced like any
    /// other source change. Absent when no membership policy is in force.
    pub membership_digest: Option<String>,
    pub boundary: ContextBoundary,
}
