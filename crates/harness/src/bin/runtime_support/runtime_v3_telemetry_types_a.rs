// SPDX-License-Identifier: MIT

//! The runtime-v3 telemetry boundary.
//!
//! This module deliberately lives beside the executable adapter. It accepts only
//! finite enums, bounded scalars, and digests of host/provider identities. The
//! worker owns the loopback socket; gameplay calls only enqueue into bounded
//! channels and never perform network I/O.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{ActionKind, DispatchStatus, EpisodeObservation, EpisodeStage, PolicyError};

const ENDPOINT: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 14318);
const OTLP_PATH: &str = "/v1/traces";
const NORMAL_QUEUE_CAPACITY: usize = 256;
const CRITICAL_QUEUE_CAPACITY: usize = 8;
const MAX_BATCH: usize = 64;
const MAX_BODY_BYTES: usize = 512 * 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TelemetryContext {
    pub(super) run_id: String,
    pub(super) episode_id: String,
    pub(super) trajectory_id: String,
    pub(super) trace_id: String,
    pub(super) instance_id: String,
    pub(super) session_id: String,
    pub(super) runtime_profile: String,
    pub(super) schema_version: String,
    pub(super) provider_revision_digest: String,
}

impl TelemetryContext {
    pub(super) fn new(
        run_id: &str,
        episode_id: &str,
        trajectory_id: &str,
        trace_id: &str,
        instance_id: &str,
        session_id: &str,
        runtime_profile: &str,
        provider_revision: &str,
    ) -> Result<Self, String> {
        for (name, value) in [
            ("run_id", run_id),
            ("episode_id", episode_id),
            ("trajectory_id", trajectory_id),
            ("trace_id", trace_id),
            ("instance_id", instance_id),
            ("session_id", session_id),
            ("runtime_profile", runtime_profile),
        ] {
            if !safe_identity(value) {
                return Err(format!("telemetry {name} is empty, unsafe, or oversized"));
            }
        }
        let identities = [run_id, episode_id, trajectory_id, trace_id];
        if identities
            .iter()
            .enumerate()
            .any(|(index, value)| identities[..index].contains(value))
        {
            return Err(String::from(
                "telemetry run, episode, trajectory, and trace identities must be distinct",
            ));
        }
        if !valid_revision(provider_revision) {
            return Err(String::from("telemetry provider revision is invalid"));
        }
        Ok(Self {
            run_id: run_id.to_owned(),
            episode_id: episode_id.to_owned(),
            trajectory_id: trajectory_id.to_owned(),
            trace_id: trace_id.to_owned(),
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            runtime_profile: runtime_profile.to_owned(),
            schema_version: String::from("runtime-v3-telemetry-1"),
            provider_revision_digest: digest("provider-revision", provider_revision),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DecisionKind {
    Action,
    Plan,
    Wait,
    Reobserve,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TelemetryActionKind {
    StartRun,
    SelectMapNode,
    PlayCard,
    EndTurn,
    ChooseReward,
    SkipReward,
    ShopPurchase,
    ShopRemove,
    Rest,
    Smith,
    EventChoice,
    SelectCard,
    ConfirmVictory,
    SaveQuit,
    Proceed,
    ConfirmSelection,
    CancelSelection,
}

impl TelemetryActionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::StartRun => "start_run",
            Self::SelectMapNode => "select_map_node",
            Self::PlayCard => "play_card",
            Self::EndTurn => "end_turn",
            Self::ChooseReward => "choose_reward",
            Self::SkipReward => "skip_reward",
            Self::ShopPurchase => "shop_purchase",
            Self::ShopRemove => "shop_remove",
            Self::Rest => "rest",
            Self::Smith => "smith",
            Self::EventChoice => "event_choice",
            Self::SelectCard => "select_card",
            Self::ConfirmVictory => "confirm_victory",
            Self::SaveQuit => "save_quit",
            Self::Proceed => "proceed",
            Self::ConfirmSelection => "confirm_selection",
            Self::CancelSelection => "cancel_selection",
        }
    }
}

impl From<ActionKind> for TelemetryActionKind {
    fn from(kind: ActionKind) -> Self {
        match kind {
            ActionKind::StartRun => Self::StartRun,
            ActionKind::SelectMapNode => Self::SelectMapNode,
            ActionKind::PlayCard => Self::PlayCard,
            ActionKind::EndTurn => Self::EndTurn,
            ActionKind::ChooseReward => Self::ChooseReward,
            ActionKind::SkipReward => Self::SkipReward,
            ActionKind::ShopPurchase => Self::ShopPurchase,
            ActionKind::ShopRemove => Self::ShopRemove,
            ActionKind::Rest => Self::Rest,
            ActionKind::Smith => Self::Smith,
            ActionKind::EventChoice => Self::EventChoice,
            ActionKind::SelectCard => Self::SelectCard,
            ActionKind::ConfirmVictory => Self::ConfirmVictory,
            ActionKind::SaveQuit => Self::SaveQuit,
            ActionKind::Proceed => Self::Proceed,
            ActionKind::ConfirmSelection => Self::ConfirmSelection,
            ActionKind::CancelSelection => Self::CancelSelection,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TelemetryStage {
    Setup,
    Map,
    Combat,
    Reward,
    Shop,
    Event,
    Rest,
    Selection,
    Victory,
    Defeat,
    Recovery,
    Unknown,
}

impl TelemetryStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Map => "map",
            Self::Combat => "combat",
            Self::Reward => "reward",
            Self::Shop => "shop",
            Self::Event => "event",
            Self::Rest => "rest",
            Self::Selection => "selection",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
            Self::Recovery => "recovery",
            Self::Unknown => "unknown",
        }
    }
}

impl From<EpisodeStage> for TelemetryStage {
    fn from(stage: EpisodeStage) -> Self {
        match stage {
            EpisodeStage::Setup => Self::Setup,
            EpisodeStage::Map => Self::Map,
            EpisodeStage::Combat => Self::Combat,
            EpisodeStage::Reward => Self::Reward,
            EpisodeStage::Shop => Self::Shop,
            EpisodeStage::Event => Self::Event,
            EpisodeStage::Rest => Self::Rest,
            EpisodeStage::Selection => Self::Selection,
            EpisodeStage::Victory => Self::Victory,
            EpisodeStage::Defeat => Self::Defeat,
            EpisodeStage::Recovery => Self::Recovery,
            EpisodeStage::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailureCode {
    InputBlocked,
    StaleCatalog,
    IllegalAction,
    MissingOperation,
    MalformedDecision,
    ProviderUnavailable,
    ProviderMalformed,
    ProviderClosed,
    Rejected,
    UnknownOutcome,
    Cleanup,
    Configuration,
    Other,
}

impl FailureCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::InputBlocked => "input_blocked",
            Self::StaleCatalog => "stale_catalog",
            Self::IllegalAction => "illegal_action",
            Self::MissingOperation => "missing_operation",
            Self::MalformedDecision => "malformed_decision",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderMalformed => "provider_malformed",
            Self::ProviderClosed => "provider_closed",
            Self::Rejected => "rejected",
            Self::UnknownOutcome => "unknown_outcome",
            Self::Cleanup => "cleanup_failed",
            Self::Configuration => "configuration_failed",
            Self::Other => "other",
        }
    }
}

impl From<&PolicyError> for FailureCode {
    fn from(error: &PolicyError) -> Self {
        match error {
            PolicyError::InputBlocked => Self::InputBlocked,
            PolicyError::StaleCatalog => Self::StaleCatalog,
            PolicyError::IllegalAction => Self::IllegalAction,
            PolicyError::MissingOperation => Self::MissingOperation,
            PolicyError::MalformedDecision => Self::MalformedDecision,
            PolicyError::ProviderUnavailable => Self::ProviderUnavailable,
            PolicyError::ProviderMalformed => Self::ProviderMalformed,
            PolicyError::ProviderClosed => Self::ProviderClosed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryKind {
    Reconnect,
    Reobserve,
    Reconcile,
}

impl RecoveryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Reobserve => "reobserve",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GameOutcome {
    Success,
    Failure,
    Unavailable,
}

impl GameOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CleanupStatus {
    Clean,
    Failed,
}

impl CleanupStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObservationSource {
    Observe,
    Reobserve,
    Transition,
    Recovery,
}

impl ObservationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Reobserve => "reobserve",
            Self::Transition => "transition",
            Self::Recovery => "recovery",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DispatchTelemetryStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

impl DispatchTelemetryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }
}

impl From<DispatchStatus> for DispatchTelemetryStatus {
    fn from(status: DispatchStatus) -> Self {
        match status {
            DispatchStatus::Accepted => Self::Accepted,
            DispatchStatus::Settled => Self::Settled,
            DispatchStatus::Rejected => Self::Rejected,
            DispatchStatus::Unknown => Self::Unknown,
            DispatchStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EventKind {
    RunStarted,
    ModelDecision,
    ModelFailure,
    Observation,
    ActionDispatch,
    SettlementObservation,
    Recovery,
    Failure,
    TerminalObserved,
    RunFinished,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::RunStarted => "run_started",
            Self::ModelDecision => "model_decision",
            Self::ModelFailure => "model_failure",
            Self::Observation => "observation",
            Self::ActionDispatch => "action_dispatch",
            Self::SettlementObservation => "settlement_observation",
            Self::Recovery => "recovery",
            Self::Failure => "failure",
            Self::TerminalObserved => "terminal_observed",
            Self::RunFinished => "run_finished",
        }
    }
}
