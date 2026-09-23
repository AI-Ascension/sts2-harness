// SPDX-License-Identifier: MIT

//! Harness-owned Phase 2 context preparation and control fencing.
//!
//! This module deliberately sits beside the existing provider ports. It prepares bytes and
//! records control decisions, while the game host remains the authority for observations, legal
//! actions, and mutations.

mod action_description;
mod derived_exact;
mod lifetime_durable;
mod lifetime_error;
mod lifetime_ledger;
mod lifetime_manifest;
mod lifetime_restore;
mod lifetime_scope;
mod lifetime_state;
mod membership;
mod membership_render;
mod model_view;
mod model_view_catalog;
mod model_view_elements;
mod model_view_error;
mod model_view_fields;
mod model_view_path;
mod model_view_projection;
mod model_view_sentinels;
mod model_view_walk;
mod option_selection;
mod render;
mod state;
mod store;
mod store_ops;
mod store_ownership;
mod store_receipts;
mod store_render_sources;
mod store_schema;
mod store_types;
mod systemone_class_request;
mod systemone_request;
mod types;

pub use action_description::describe_action;
pub use derived_exact::{DERIVED_EXACT_SCHEMA, DerivedExactFacts, Survival};
pub use lifetime_durable::{DURABLE_LIFETIME_SCHEMA, DurableLifetimeState};
pub use lifetime_error::ContextLifetimeError;
pub use lifetime_ledger::{ContextLifetimeLedger, LifetimeFailpoint};
pub use lifetime_manifest::{
    DispatchSettlement, LifetimeApproval, LifetimeManifest, MAX_LIFETIME_MANIFEST_BYTES,
};
pub use lifetime_scope::{
    CONTEXT_LIFETIME_SCHEMA, ContextLifetimeScope, InvocationOwnerScope, LifetimeApplicability,
    LogicalInvocationIdentity, MAX_LIFETIME_ITEMS, MAX_LIFETIME_MANIFESTS, MAX_LIFETIME_NEXT_N,
    MAX_LIFETIME_SCOPES,
};
pub use lifetime_state::LifetimePreview;
pub use membership::{
    CONTEXT_MEMBERSHIP_POLICY_SCHEMA, ContextMembershipBroaderScope, ContextMembershipError,
    ContextMembershipPolicy, ContextMembershipScope, ContextMembershipSelector, ContextModelView,
    EffectiveMembership, MAX_MEMBERSHIP_AUTHORIZED_AGENTS, MAX_MEMBERSHIP_WIDER_ITEMS,
    MembershipCheckContext, MembershipContinuity, MembershipDecision, MembershipDispatchView,
    MembershipDisposition, MembershipReasonCode, PreparedMembership, SHARED_MEMBERSHIP_KINDS,
    prevalidate_and_bind, resolve_membership,
};
pub use membership_render::{
    MembershipRenderError, MembershipRenderRequest, membership_check_from_boundary,
    membership_scope_from_boundary, render_with_membership,
};
pub use model_view::{
    ALL_ITEMS_MARKER, MODEL_VIEW_PROJECTION_SCHEMA, ModelViewProjection, ModelViewSelectorRegistry,
    PathSegment, ProjectedField, ReadStep, ViewFieldPath,
};
pub use model_view_catalog::{
    ElementKind, FIELD_CATALOG, FieldPresence, FieldProtection, FieldShape, FieldSpec, FieldType,
    MAX_MODEL_VIEW_FIELDS, MAX_MODEL_VIEW_PATH_SEGMENTS, MAX_PROJECTED_COLLECTION,
    ModelFieldMetadata, ViewContext, catalog_metadata, declared_names, spec,
};
pub use model_view_error::ModelViewProjectionError;
pub use model_view_path::display_path;
pub use model_view_projection::{
    AdmittedSourceObservation, MAX_MODEL_VIEW_BYTES, ModelViewApproval, PreparedModelView,
    project_model_view,
};
pub use model_view_sentinels::{
    catalog_paths, excluded_sentinel_paths, fair_play_verdict, reject_excluded_sentinels,
};
pub use option_selection::{
    MAX_PRESENTED_OPTIONS, OPTION_SELECTION_SCHEMA, OptionSelection, PresentedOption,
    SelectionMode, WithheldOption, WithheldReason,
};
pub use render::{
    ContextRenderError, ContextRenderLimits, ContextRenderer, ManagedRenderInput, PreparedContext,
    ollama_user_content,
};
pub use state::{
    ControlAuthority, ControlEvent, ControlReceipt, ControlState, GateStatus, MAX_CONTROL_EVENTS,
};
pub use store::ContextControlStore;
pub use store_render_sources::context_source_digest;
pub use store_types::{
    CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION, DurableActiveContextSource,
    DurableContextOwnerControlReceipt, DurableContextSourceSnapshot, DurableControlStoreError,
    DurableStoreFailpoint, LegacyOpenError, StoreMode, StoreSnapshot,
};
pub use systemone_class_request::{KIND_QUESTION, build_class_system_one_request};
pub use systemone_request::{
    ACTION_QUESTION, MAX_DESCRIPTION_BYTES, MAX_OPTIONS, MAX_STATE_AND_QUESTION_BYTES,
    SYSTEM_ONE_PATH, SystemOneOption, SystemOneRequestError, build_described_system_one_request,
    build_system_one_request, system_one_questions_digest,
};
pub use types::{
    ActiveContextSource, CONTEXT_DRAFT_SCHEMA, ContextBoundary, ContextDraft, ContextItem,
    ContextItemRef, ContextNote, ContextSourceActivation, ContextSourceDocument, MAX_CONTEXT_BYTES,
    MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES, MAX_OBJECTIVE_BYTES, ManagementProfile,
};
