// SPDX-License-Identifier: MIT

//! In-memory state of one admitted live run.
//!
//! The coordinator in [`super::execution`] owns only lookup; each run owns its
//! session handle, admitted definition digest and bounded dispatch state.

use super::super::context_owner::ContextOwnerBinding;
use super::super::contract::{CleanupState, PendingOperationState, TargetAdmissionBinding};
use super::execution_context::ContextNode;
use super::session::{LiveWorkflowOptions, LiveWorkflowSession};
use crate::episode::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    TransitionReceipt,
};
use crate::workflow::StrictRuntime;

pub(super) struct LiveRun {
    pub(super) runtime: StrictRuntime,
    pub(super) definition_digest: String,
    pub(super) run_id: String,
    pub(super) instance_id: String,
    pub(super) state: LiveNodeState,
    pub(super) cancelled: bool,
    pub(super) cleanup: CleanupState,
    pub(super) admission: Option<TargetAdmissionBinding>,
    /// Context-bound invocations declared by the admitted definition. Used to
    /// decide, at dispatch time, which node must be bound by the owner.
    pub(super) context_nodes: Vec<ContextNode>,
    /// The owner binding accepted for the most recently dispatched
    /// context-bound node, retained as bounded admission evidence.
    pub(super) context_binding: Option<ContextOwnerBinding>,
}

pub(super) struct LiveNodeState {
    pub(super) session: Box<dyn LiveWorkflowSession>,
    pub(super) instance_id: String,
    pub(super) observation: Option<EpisodeObservation>,
    pub(super) actions: Option<EpisodeLegalActionSet>,
    pub(super) pending: Option<PendingDispatch>,
    pub(super) provider_calls: u64,
    pub(super) max_provider_calls: u64,
    pub(super) options: LiveWorkflowOptions,
}

pub(super) struct PendingDispatch {
    pub(super) identity: ActionIdentity,
    pub(super) action: EpisodeLegalAction,
    pub(super) state: PendingOperationState,
    pub(super) resolved: Option<TransitionReceipt>,
}
