// SPDX-License-Identifier: MIT

//! Durable draft, eligible-item, revision, preview, and mutation-receipt methods for the
//! production owner. This module only consumes bytes admitted by the owner's immutable source
//! catalog and the host-thread render snapshot captured by `source_render`.

use super::source::{source_valid_until, unix_time, validate_document};
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sts2_harness::context_control::{
    ContextDraft, ContextItem, ContextItemRef, ContextNote, ContextSourceDocument,
    DurableOwnerContextState, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES,
    MAX_NOTE_BYTES, MAX_OBJECTIVE_BYTES,
};
use sts2_harness::management::{
    CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION, CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION,
    CONTEXT_OWNER_DRAFT_SCHEMA_VERSION, CONTEXT_OWNER_ITEMS_SCHEMA_VERSION,
    CONTEXT_OWNER_MUTATION_RECEIPT_SCHEMA_VERSION, CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION,
    CONTEXT_OWNER_PREVIEW_SCHEMA_VERSION, CONTEXT_OWNER_REVISION_SCHEMA_VERSION,
    ContextOwnerDraftCreateRequest, ContextOwnerDraftEnvelope, ContextOwnerDraftListView,
    ContextOwnerDraftOperation, ContextOwnerDraftPatchRequest, ContextOwnerDraftPublicationRequest,
    ContextOwnerItemView, ContextOwnerItemsView, ContextOwnerMutationReceipt,
    ContextOwnerMutationRequest, ContextOwnerMutationResult, ContextOwnerPreviewEnvelope,
    ContextOwnerPreviewRequest, ContextOwnerRevisionEnvelope, ContextOwnerRevisionPage,
    MAX_CONTEXT_OWNER_PAGE_SIZE, validate_identifier,
};

const OWNER_STATE_SCHEMA: &str = "ascension.harness.context-owner-state.v1";
const MAX_DRAFTS: usize = 32;
const MAX_REVISIONS: usize = 128;
const MAX_PREVIEWS: usize = 64;
const MAX_RECEIPTS_PER_ACTOR: usize = 128;
const MAX_AUTHORED_ITEMS: usize = 256;
const PREVIEW_TTL_SECONDS: u64 = 300;

include!("drafts/model.rs");
include!("drafts/common.rs");
include!("drafts/state.rs");
include!("drafts/publication.rs");
include!("drafts/access.rs");
include!("drafts/create.rs");
include!("drafts/patch.rs");
include!("drafts/patch/operations.rs");
include!("drafts/patch/authored_operations.rs");
include!("drafts/patch/validation.rs");
include!("drafts/preview.rs");
include!("drafts/recovery.rs");
include!("drafts/mutation_helpers.rs");
include!("drafts/helpers.rs");

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
