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
    pub(super) pending_decision: Option<PendingDecision>,
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

/// One held provider-decision attempt.
///
/// This is the decide-node counterpart of [`PendingDispatch`]: the intent is installed before the
/// provider exchange and released only once a usable decision for this exact invocation exists, so
/// a lost reply, an operator step or a restart cannot become a second paid exchange. `execution_id`
/// and `input_digest` are the admitted request identity; a retry that does not reproduce both
/// exactly may never reuse the attempt.
pub(super) struct PendingDecision {
    pub(super) operation_id: String,
    pub(super) execution_id: u64,
    pub(super) generation: u64,
    pub(super) input_digest: String,
    pub(super) state: PendingOperationState,
    pub(super) resolved: Option<Box<crate::Decision>>,
}
