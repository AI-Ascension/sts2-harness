// SPDX-License-Identifier: MIT

use super::super::identity::{
    CoopNativeActionId, CoopNativeAuthorityId, CoopNativeCheckpointId, CoopNativeCorrelationId,
    CoopNativeEffectId, CoopNativeInstanceId, CoopNativeLeaseId, CoopNativeOperationId,
    CoopNativePeerId, CoopNativeProposalId, CoopNativeRunId, CoopNativeSessionId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeKind {
    Observation,
    LegalCatalogRequest,
    LegalCatalogResponse,
    LocalActionRequest,
    SharedVoteRequest,
    RejoinRequest,
    EffectResponse,
    RecoveryResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeActionKind {
    PlayCard,
    EndTurn,
    SelectCard,
    ChooseReward,
    ConfirmSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativePeerRole {
    Local,
    Ally,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeChecksumStatus {
    Available,
    Unavailable,
    Unknown,
    Disabled,
    EnabledUnread,
    Enabled,
    Divergent,
    Matched,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeEffectKind {
    TurnEnded,
    SharedEventVote,
    NativeEndTurnSettled,
    NativePlayCardSettled,
    NativeSharedEventVoteSettled,
    NativeTreasureRelicVoteSettled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeRecoveryKind {
    Reconcile,
    Rejoin,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeHeader {
    pub(super) correlation_id: CoopNativeCorrelationId,
    pub(super) instance_id: CoopNativeInstanceId,
    pub(super) session_id: CoopNativeSessionId,
    pub(super) lease_id: CoopNativeLeaseId,
    pub(super) lease_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeAction {
    pub(super) kind: CoopNativeActionKind,
    pub(super) action_id: CoopNativeActionId,
    pub(super) target_peer: Option<CoopNativePeerId>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeVote {
    pub(super) proposal_id: CoopNativeProposalId,
    pub(super) voter_peer: CoopNativePeerId,
    pub(super) choice: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeRecovery {
    pub(super) kind: CoopNativeRecoveryKind,
    pub(super) rejoin_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativePeerSnapshot {
    pub(super) peer_token: CoopNativePeerId,
    pub(super) authority_id: CoopNativeAuthorityId,
    pub(super) role: CoopNativePeerRole,
    pub(super) connected: bool,
    pub(super) peer_generation: u64,
    pub(super) state_digest: String,
    pub(super) rejoin_epoch: u64,
    pub(super) authority_epoch: CoopNativeAuthorityId,
    pub(super) checkpoint_id: Option<CoopNativeCheckpointId>,
    pub(super) digest_known: bool,
    pub(super) is_loading: bool,
    pub(super) is_divergent: bool,
    pub(super) checksum_status: CoopNativeChecksumStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeObservation {
    pub(super) host_authority_epoch: CoopNativeAuthorityId,
    pub(super) authority_id: CoopNativeAuthorityId,
    pub(super) run_id: CoopNativeRunId,
    pub(super) host_sequence_kind: String,
    pub(super) host_generation: u64,
    pub(super) state_digest: String,
    pub(super) checkpoint_id: CoopNativeCheckpointId,
    pub(super) checksum_algorithm: String,
    pub(super) checksum_status: CoopNativeChecksumStatus,
    pub(super) native_checksum: Option<String>,
    pub(super) host_digest_known: bool,
    pub(super) host_loading: bool,
    pub(super) host_divergent: bool,
    pub(super) peers: Vec<CoopNativePeerSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeEffect {
    pub(super) effect_id: CoopNativeEffectId,
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) kind: CoopNativeEffectKind,
    pub(super) from_generation: u64,
    pub(super) to_generation: u64,
    pub(super) state_digest: String,
    pub(super) authority_epoch: CoopNativeAuthorityId,
    pub(super) checkpoint_id: CoopNativeCheckpointId,
    pub(super) authority_id: CoopNativeAuthorityId,
    pub(super) native_checksum: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeReceipt {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) status: CoopNativeStatus,
    pub(super) before_host_generation: u64,
    pub(super) after_host_generation: Option<u64>,
    pub(super) authority_id: CoopNativeAuthorityId,
    pub(super) authority_epoch: CoopNativeAuthorityId,
    pub(super) checkpoint_id: CoopNativeCheckpointId,
    pub(super) state_digest: String,
    pub(super) native_checksum: Option<String>,
    pub(super) error_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeCatalog {
    pub(super) host_generation: u64,
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) actions: Vec<CoopNativeAction>,
    pub(super) votes: Vec<CoopNativeVote>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeLocalActionRequest {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) expected_host_generation: u64,
    pub(super) action: CoopNativeAction,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeSharedVoteRequest {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) expected_host_generation: u64,
    pub(super) vote: CoopNativeVote,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeRejoinRequest {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) expected_host_generation: u64,
    pub(super) recovery: CoopNativeRecovery,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeLegalCatalogRequest {
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) expected_host_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeLegalCatalogResponse {
    pub(super) actor_peer: CoopNativePeerId,
    pub(super) expected_host_generation: u64,
    pub(super) observation: CoopNativeObservation,
    pub(super) catalog: CoopNativeCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeEffectResponse {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) status: CoopNativeStatus,
    pub(super) observation: CoopNativeObservation,
    pub(super) effect: Option<CoopNativeEffect>,
    pub(super) receipt: CoopNativeReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeRecoveryResponse {
    pub(super) operation_id: CoopNativeOperationId,
    pub(super) status: Option<CoopNativeStatus>,
    pub(super) observation: Option<CoopNativeObservation>,
    pub(super) recovery: CoopNativeRecovery,
    pub(super) receipt: Option<CoopNativeReceipt>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(super) enum CoopNativeBody {
    Observation(CoopNativeObservation),
    LegalCatalogRequest(CoopNativeLegalCatalogRequest),
    LegalCatalogResponse(CoopNativeLegalCatalogResponse),
    LocalActionRequest(CoopNativeLocalActionRequest),
    SharedVoteRequest(CoopNativeSharedVoteRequest),
    RejoinRequest(CoopNativeRejoinRequest),
    EffectResponse(CoopNativeEffectResponse),
    RecoveryResponse(CoopNativeRecoveryResponse),
}

include!("coop_native_wire_accessors.rs");
