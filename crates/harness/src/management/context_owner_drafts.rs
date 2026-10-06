// SPDX-License-Identifier: MIT

//! Closed, bounded wire records for the Harness-owned draft surface.

use serde::{Deserialize, Serialize};

use super::ContextOwnerBinding;
use crate::context_control::{ContextBoundary, ContextDraft, ContextItemRef};

pub const CONTEXT_OWNER_DRAFT_SCHEMA_VERSION: &str = "ascension.harness.context-owner-draft.v1";
pub const CONTEXT_OWNER_REVISION_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-revision.v1";
pub const CONTEXT_OWNER_PREVIEW_SCHEMA_VERSION: &str = "ascension.harness.context-owner-preview.v1";
pub const CONTEXT_OWNER_MUTATION_RECEIPT_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-mutation-receipt.v1";
pub const CONTEXT_OWNER_ITEMS_SCHEMA_VERSION: &str = "ascension.harness.context-owner-items.v1";
pub const CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-draft-request.v1";
pub const CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-draft-patch.v1";
pub const CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-preview-request.v1";
pub const CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION: &str =
    "ascension.harness.context-owner-mutation-lookup.v1";
pub const MAX_CONTEXT_OWNER_PAGE_SIZE: u64 = 50;
pub const MAX_CONTEXT_OWNER_DRAFT_OPERATIONS: usize = 32;

/// One owner-resolved eligible item. Content is present only for the separately
/// authorized content projection; identity and metadata never grant eligibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerItemView {
    pub reference: ContextItemRef,
    pub kind: String,
    pub byte_length: u64,
    pub protected: bool,
    pub expires_at: u64,
    pub source_id: String,
    pub source_version: u64,
    pub source_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerItemsView {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub binding_id: String,
    pub binding_digest: String,
    pub boundary: ContextBoundary,
    pub items: Vec<ContextOwnerItemView>,
}

/// Immutable metadata envelope around an owner-controlled mutable draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftEnvelope {
    pub schema_version: String,
    pub actor_subject: String,
    pub binding: ContextOwnerBinding,
    pub created_at: u64,
    pub updated_at: u64,
    /// Finite expiry inherited from this draft's exact active base source, when one exists.
    /// Authored bytes are refused when no such owner horizon is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_expires_at: Option<u64>,
    pub draft: ContextDraft,
}

/// One durable immutable snapshot of a successfully created or patched draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerRevisionEnvelope {
    pub schema_version: String,
    pub revision_id: String,
    pub actor_subject: String,
    pub binding: ContextOwnerBinding,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_expires_at: Option<u64>,
    pub draft: ContextDraft,
}

/// Metadata-only result of rendering the exact current owner boundary and draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerPreviewEnvelope {
    pub schema_version: String,
    pub preview_id: String,
    pub actor_subject: String,
    pub binding: ContextOwnerBinding,
    pub provider_config_digest: String,
    pub draft_id: String,
    pub draft_version: u64,
    pub base_revision_id: String,
    pub manifest_digest: String,
    pub effect_class: String,
    pub blockers: Vec<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftCreateRequest {
    pub schema_version: String,
    pub request_id: String,
    pub draft_id: String,
    pub base_revision_id: String,
    pub expected_boundary: ContextBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextOwnerDraftOperation {
    IncludeItem { reference: ContextItemRef },
    ExcludeItem { reference: ContextItemRef },
    PinItem { item_id: String },
    UnpinItem { item_id: String },
    PutNote { note_id: String, text: String },
    RemoveNote { note_id: String },
    SetObjective { text: String },
    RemoveObjective,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerDraftPatchRequest {
    pub schema_version: String,
    pub request_id: String,
    pub draft_id: String,
    pub expected_version: u64,
    pub expected_boundary: ContextBoundary,
    pub operations: Vec<ContextOwnerDraftOperation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerPreviewRequest {
    pub schema_version: String,
    pub request_id: String,
    pub draft_id: String,
    pub expected_version: u64,
    pub expected_boundary: ContextBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ContextOwnerMutationRequest {
    CreateDraft(ContextOwnerDraftCreateRequest),
    PatchDraft(ContextOwnerDraftPatchRequest),
    CreatePreview(ContextOwnerPreviewRequest),
}

impl ContextOwnerMutationRequest {
    #[must_use]
    pub fn request_id(&self) -> &str {
        match self {
            Self::CreateDraft(request) => &request.request_id,
            Self::PatchDraft(request) => &request.request_id,
            Self::CreatePreview(request) => &request.request_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ContextOwnerMutationResult {
    Draft(ContextOwnerDraftEnvelope),
    Preview(ContextOwnerPreviewEnvelope),
}

/// Terminal result and the exact identity required to recover it after a lost reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerMutationReceipt {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub actor_subject: String,
    pub binding_id: String,
    pub binding_digest: String,
    pub invocation_id: String,
    pub boundary: ContextBoundary,
    pub operation: String,
    pub request_id: String,
    pub payload_digest: String,
    pub result: ContextOwnerMutationResult,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerMutationLookupRequest {
    pub schema_version: String,
    pub request: ContextOwnerMutationRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContextOwnerDraftListView {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub drafts: Vec<ContextOwnerDraftEnvelope>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContextOwnerRevisionPage {
    pub schema_version: String,
    pub owner_id: String,
    pub workflow_run_id: String,
    pub revisions: Vec<ContextOwnerRevisionEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_after_revision_id: Option<String>,
}
