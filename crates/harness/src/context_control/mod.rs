// SPDX-License-Identifier: MIT

//! Harness-owned Phase 2 context preparation and control fencing.
//!
//! This module deliberately sits beside the existing provider ports. It prepares bytes and
//! records control decisions, while the game host remains the authority for observations, legal
//! actions, and mutations.

mod render;
mod state;
mod store;
mod store_ops;
mod store_ownership;
mod store_schema;
mod store_types;
mod types;

pub use render::{
    ContextRenderError, ContextRenderLimits, ContextRenderer, ManagedRenderInput, PreparedContext,
    ollama_user_content,
};
pub use state::{
    ControlAuthority, ControlEvent, ControlReceipt, ControlState, GateStatus, MAX_CONTROL_EVENTS,
};
pub use store::ContextControlStore;
pub use store_types::{
    CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION, DurableControlStoreError, DurableStoreFailpoint,
    LegacyOpenError, StoreMode, StoreSnapshot,
};
pub use types::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextNote, MAX_CONTEXT_BYTES,
    MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES, MAX_OBJECTIVE_BYTES, ManagementProfile,
};
