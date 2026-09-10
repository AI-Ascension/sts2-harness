// SPDX-License-Identifier: MIT

//! Harness-owned Phase 2 context preparation and control fencing.
//!
//! This module deliberately sits beside the existing provider ports. It prepares bytes and
//! records control decisions, while the game host remains the authority for observations, legal
//! actions, and mutations.

mod render;
mod state;
mod types;

pub use render::{
    ContextRenderError, ContextRenderer, ManagedRenderInput, PreparedContext, ollama_user_content,
};
pub use state::{ControlAuthority, ControlEvent, ControlReceipt, ControlState, GateStatus};
pub use types::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextNote, ManagementProfile,
};
