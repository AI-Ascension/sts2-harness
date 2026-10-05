// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::{fs, path::PathBuf};
use sts2_harness::context_control::{
    ContextBoundary, ContextDraft, ContextItem, ContextSourceDocument, ControlAuthority, StoreMode,
    context_source_digest,
};
use sts2_harness::management::{
    Authenticator, Budget, CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION,
    CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION, CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION,
    CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION, CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION,
    CleanupState, ContextBindingRequest, ContextBindingSource, ContextControlCommand,
    ContextOwnerDraftCreateRequest, ContextOwnerDraftOperation, ContextOwnerDraftPatchRequest,
    ContextOwnerMutationLookupRequest, ContextOwnerMutationReceipt, ContextOwnerMutationRequest,
    ContextOwnerMutationResult, ContextOwnerPreviewRequest, ContextSourceAdoptionRequest, Cursor,
    EVENT_SCHEMA_VERSION, EventClassification, EventPayload, EventType, GameOutcome,
    MANAGEMENT_SCHEMA_VERSION, ManagementServer, ManagementService, MemoryWorkflowStore,
    RUN_SCHEMA_VERSION, RunEvent, RunSnapshot, ServerConfig, StaticAuthenticator,
    WorkflowRunStatus, WorkflowStore,
};
use sts2_harness::{
    ActionKind, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeStage, ExoConfig, ModelExecutionId,
};

const OWNER_KEY: [u8; 32] = [0x47; 32];
const SUBJECT: &str = "draft-operator";

struct OwnerFixture {
    directory: PathBuf,
    owner: Arc<Owner>,
    configuration: Configuration,
    workflow_store: Arc<MemoryWorkflowStore>,
    request: RunRequest,
    snapshot: RunSnapshot,
    actor: AuthContext,
    runtime_binding: RuntimeAuthorityBinding,
    control_limits: ContextOwnerControlLimits,
    input: DecisionInput,
}

include!("drafts_tests/fixture.rs");
include!("drafts_tests/http_support.rs");
include!("drafts_tests/served_flow.rs");
include!("drafts_tests/grant_boundaries.rs");
include!("drafts_tests/render_identity.rs");
