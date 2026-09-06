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

#[derive(Clone, Debug)]
enum TelemetryEvent {
    RunStarted,
    ModelDecision {
        model_execution_id: u64,
        decision_kind: DecisionKind,
        action_id_digest: Option<String>,
        operation_id_digest: Option<String>,
        confidence: Option<u8>,
    },
    ModelFailure {
        model_execution_id: u64,
        failure_code: FailureCode,
    },
    Observation {
        source: ObservationSource,
        generation: u64,
        stage: TelemetryStage,
        state_id_digest: String,
    },
    ActionDispatch {
        operation_id_digest: String,
        action_id_digest: String,
        action_kind: TelemetryActionKind,
        generation: u64,
        status: DispatchTelemetryStatus,
        failure_code: Option<FailureCode>,
    },
    SettlementObservation {
        operation_id_digest: String,
        action_id_digest: String,
        from_generation: u64,
        to_generation: u64,
        stage: TelemetryStage,
        effect_class: &'static str,
        effect_digest: String,
        source: ObservationSource,
    },
    Recovery {
        kind: RecoveryKind,
        operation_id_digest: Option<String>,
        attempt: u8,
        outcome: &'static str,
        failure_code: Option<FailureCode>,
    },
    Failure {
        boundary: &'static str,
        failure_code: FailureCode,
        retryable: bool,
        operation_id_digest: Option<String>,
    },
    TerminalObserved {
        stage: TelemetryStage,
        outcome: GameOutcome,
        generation: u64,
        state_id_digest: String,
    },
    RunFinished {
        outcome: GameOutcome,
        terminal_stage: TelemetryStage,
        cleanup_status: CleanupStatus,
        dropped_events: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EnqueueStatus {
    Queued,
    Dropped,
    Closed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FlushReport {
    pub(super) sent: u64,
    pub(super) failed: u64,
    pub(super) normal_dropped: u64,
    pub(super) critical_dropped: u64,
    pub(super) timed_out: bool,
}

impl FlushReport {
    pub(super) fn export_status(&self) -> &'static str {
        if self.timed_out {
            "timeout"
        } else if self.failed > 0 || self.normal_dropped > 0 || self.critical_dropped > 0 {
            "partial"
        } else {
            "delivered"
        }
    }
}

struct ExporterState {
    context: TelemetryContext,
    normal_tx: SyncSender<TelemetryEvent>,
    critical_tx: SyncSender<TelemetryEvent>,
    closed: AtomicBool,
    sequence: AtomicU64,
    normal_dropped: AtomicU64,
    critical_dropped: AtomicU64,
}

#[derive(Clone)]
pub(super) struct TelemetryHandle {
    state: Arc<ExporterState>,
}

pub(super) struct RuntimeV3Telemetry {
    handle: TelemetryHandle,
    control_tx: SyncSender<ControlMessage>,
    worker: Option<JoinHandle<WorkerReport>>,
}

enum ControlMessage {
    Flush(SyncSender<WorkerReport>),
}

#[derive(Clone, Debug, Default)]
struct WorkerReport {
    sent: u64,
    failed: u64,
}

impl RuntimeV3Telemetry {
    pub(super) fn new(context: TelemetryContext) -> Self {
        let (normal_tx, normal_rx) = mpsc::sync_channel(NORMAL_QUEUE_CAPACITY);
        let (critical_tx, critical_rx) = mpsc::sync_channel(CRITICAL_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::sync_channel(1);
        let state = Arc::new(ExporterState {
            context,
            normal_tx,
            critical_tx,
            closed: AtomicBool::new(false),
            sequence: AtomicU64::new(1),
            normal_dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
        });
        let worker_state = Arc::clone(&state);
        let worker = thread::Builder::new()
            .name(String::from("sts2-telemetry"))
            .spawn(move || worker_loop(worker_state, normal_rx, critical_rx, control_rx))
            .ok();
        Self {
            handle: TelemetryHandle { state },
            control_tx,
            worker,
        }
    }

    pub(super) fn handle(&self) -> TelemetryHandle {
        self.handle.clone()
    }

    pub(super) fn finish(mut self, deadline: Duration) -> FlushReport {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let control_status = self.control_tx.try_send(ControlMessage::Flush(reply_tx));
        let mut report = match control_status {
            Ok(()) => match reply_rx.recv_timeout(deadline) {
                Ok(worker) => FlushReport {
                    sent: worker.sent,
                    failed: worker.failed,
                    normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                    critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                    timed_out: false,
                },
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => FlushReport {
                    normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                    critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                    timed_out: true,
                    ..FlushReport::default()
                },
            },
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => FlushReport {
                normal_dropped: self.handle.state.normal_dropped.load(Ordering::Relaxed),
                critical_dropped: self.handle.state.critical_dropped.load(Ordering::Relaxed),
                timed_out: true,
                ..FlushReport::default()
            },
        };
        self.handle.state.closed.store(true, Ordering::Release);
        let normal_dropped = self.handle.state.normal_dropped.load(Ordering::Relaxed);
        let critical_dropped = self.handle.state.critical_dropped.load(Ordering::Relaxed);
        drop(self.handle);
        drop(self.control_tx);
        if let Some(worker) = self.worker.take() {
            if report.timed_out {
                // `JoinHandle::join` has no deadline. The worker owns only a loopback
                // socket with bounded I/O timeouts, so detach it after a timed-out
                // flush rather than extending the gameplay shutdown indefinitely.
                drop(worker);
            } else if worker.join().is_err() {
                report.failed = report.failed.saturating_add(1);
            }
        }
        report.normal_dropped = report.normal_dropped.max(normal_dropped);
        report.critical_dropped = report.critical_dropped.max(critical_dropped);
        report
    }
}

impl TelemetryHandle {
    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        let (normal_tx, _normal_rx) = mpsc::sync_channel(0);
        let (critical_tx, _critical_rx) = mpsc::sync_channel(0);
        let state = ExporterState {
            context: TelemetryContext {
                run_id: String::from("disabled-run"),
                episode_id: String::from("disabled-episode"),
                trajectory_id: String::from("disabled-trajectory"),
                trace_id: String::from("disabled-trace"),
                instance_id: String::from("disabled-instance"),
                session_id: String::from("disabled-session"),
                runtime_profile: String::from("disabled"),
                schema_version: String::from("disabled"),
                provider_revision_digest: String::from("disabled"),
            },
            normal_tx,
            critical_tx,
            closed: AtomicBool::new(true),
            sequence: AtomicU64::new(1),
            normal_dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
        };
        Self {
            state: Arc::new(state),
        }
    }

    pub(super) fn run_started(&self) -> EnqueueStatus {
        self.enqueue(TelemetryEvent::RunStarted, true)
    }

    pub(super) fn model_decision(
        &self,
        model_execution_id: u64,
        decision_kind: DecisionKind,
        action_id: Option<&str>,
        operation_id: Option<&str>,
        confidence: Option<u8>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ModelDecision {
                model_execution_id,
                decision_kind,
                action_id_digest: action_id.map(|value| digest("action", value)),
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
                confidence: confidence.filter(|value| *value <= 100),
            },
            false,
        )
    }

    pub(super) fn model_failure(
        &self,
        model_execution_id: u64,
        failure_code: FailureCode,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ModelFailure {
                model_execution_id,
                failure_code,
            },
            false,
        )
    }

    pub(super) fn observation(
        &self,
        source: ObservationSource,
        observation: &EpisodeObservation,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Observation {
                source,
                generation: observation.generation(),
                stage: observation.stage().into(),
                state_id_digest: digest("state", observation.state_id()),
            },
            false,
        )
    }

    pub(super) fn action_dispatch(
        &self,
        operation_id: &str,
        action_id: &str,
        action_kind: ActionKind,
        generation: u64,
        status: DispatchStatus,
        failure_code: Option<FailureCode>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::ActionDispatch {
                operation_id_digest: digest("operation", operation_id),
                action_id_digest: digest("action", action_id),
                action_kind: action_kind.into(),
                generation,
                status: status.into(),
                failure_code,
            },
            false,
        )
    }

    pub(super) fn settlement(
        &self,
        operation_id: &str,
        action_id: &str,
        from_generation: u64,
        after: &EpisodeObservation,
        effect_kind: &str,
        source: ObservationSource,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::SettlementObservation {
                operation_id_digest: digest("operation", operation_id),
                action_id_digest: digest("action", action_id),
                from_generation,
                to_generation: after.generation(),
                stage: after.stage().into(),
                effect_class: effect_class(effect_kind),
                effect_digest: digest("effect", effect_kind),
                source,
            },
            false,
        )
    }

    pub(super) fn recovery(
        &self,
        kind: RecoveryKind,
        operation_id: Option<&str>,
        attempt: u8,
        outcome: &'static str,
        failure_code: Option<FailureCode>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Recovery {
                kind,
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
                attempt,
                outcome,
                failure_code,
            },
            false,
        )
    }

    pub(super) fn failure(
        &self,
        boundary: &'static str,
        failure_code: FailureCode,
        retryable: bool,
        operation_id: Option<&str>,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::Failure {
                boundary,
                failure_code,
                retryable,
                operation_id_digest: operation_id.map(|value| digest("operation", value)),
            },
            false,
        )
    }

    pub(super) fn terminal(
        &self,
        observation: &EpisodeObservation,
        outcome: GameOutcome,
    ) -> EnqueueStatus {
        self.enqueue(
            TelemetryEvent::TerminalObserved {
                stage: observation.stage().into(),
                outcome,
                generation: observation.generation(),
                state_id_digest: digest("state", observation.state_id()),
            },
            true,
        )
    }

    pub(super) fn run_finished(
        &self,
        outcome: GameOutcome,
        terminal_stage: TelemetryStage,
        cleanup_status: CleanupStatus,
    ) -> EnqueueStatus {
        let dropped = self
            .state
            .normal_dropped
            .load(Ordering::Relaxed)
            .saturating_add(self.state.critical_dropped.load(Ordering::Relaxed));
        self.enqueue(
            TelemetryEvent::RunFinished {
                outcome,
                terminal_stage,
                cleanup_status,
                dropped_events: dropped,
            },
            true,
        )
    }

    fn enqueue(&self, event: TelemetryEvent, critical: bool) -> EnqueueStatus {
        if self.state.closed.load(Ordering::Acquire) {
            return EnqueueStatus::Closed;
        }
        let result = if critical {
            self.state.critical_tx.try_send(event)
        } else {
            self.state.normal_tx.try_send(event)
        };
        match result {
            Ok(()) => EnqueueStatus::Queued,
            Err(TrySendError::Full(_)) => {
                if critical {
                    self.state.critical_dropped.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.state.normal_dropped.fetch_add(1, Ordering::Relaxed);
                }
                EnqueueStatus::Dropped
            }
            Err(TrySendError::Disconnected(_)) => EnqueueStatus::Closed,
        }
    }
}

fn worker_loop(
    state: Arc<ExporterState>,
    normal_rx: Receiver<TelemetryEvent>,
    critical_rx: Receiver<TelemetryEvent>,
    control_rx: Receiver<ControlMessage>,
) -> WorkerReport {
    let mut report = WorkerReport::default();
    loop {
        if let Ok(control) = control_rx.try_recv() {
            match control {
                ControlMessage::Flush(reply) => {
                    drain_events(&state, &normal_rx, &critical_rx, &mut report);
                    let _ = reply.send(report.clone());
                    break;
                }
            }
        }
        let event = receive_event(&normal_rx, &critical_rx);
        if let Some(event) = event {
            let mut events = vec![event];
            while events.len() < MAX_BATCH {
                match critical_rx.try_recv().or_else(|_| normal_rx.try_recv()) {
                    Ok(next) => events.push(next),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            if send_batch(&state, &events) {
                report.sent = report.sent.saturating_add(events.len() as u64);
            } else {
                report.failed = report.failed.saturating_add(events.len() as u64);
            }
        }
    }
    report
}

fn receive_event(
    normal_rx: &Receiver<TelemetryEvent>,
    critical_rx: &Receiver<TelemetryEvent>,
) -> Option<TelemetryEvent> {
    match critical_rx.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Disconnected) => normal_rx.try_recv().ok(),
        Err(TryRecvError::Empty) => match normal_rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Disconnected) => {
                critical_rx.recv_timeout(Duration::from_millis(25)).ok()
            }
            Err(TryRecvError::Empty) => match critical_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => {
                    normal_rx.recv_timeout(Duration::from_millis(25)).ok()
                }
                Err(RecvTimeoutError::Disconnected) => {
                    normal_rx.recv_timeout(Duration::from_millis(25)).ok()
                }
            },
        },
    }
}

fn drain_events(
    state: &ExporterState,
    normal_rx: &Receiver<TelemetryEvent>,
    critical_rx: &Receiver<TelemetryEvent>,
    report: &mut WorkerReport,
) {
    loop {
        let mut events = Vec::new();
        while events.len() < MAX_BATCH {
            match critical_rx.try_recv().or_else(|_| normal_rx.try_recv()) {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if events.is_empty() {
            return;
        }
        if send_batch(state, &events) {
            report.sent = report.sent.saturating_add(events.len() as u64);
        } else {
            report.failed = report.failed.saturating_add(events.len() as u64);
        }
    }
}

fn send_batch(state: &ExporterState, events: &[TelemetryEvent]) -> bool {
    let mut spans = Vec::with_capacity(events.len());
    for event in events {
        let sequence = state.sequence.fetch_add(1, Ordering::Relaxed);
        spans.push(render_span(&state.context, event, sequence));
    }
    let body = json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": "sts2-harness"}},
                {"key": "service.version", "value": {"stringValue": "runtime-v3"}},
                {"key": "deployment.environment", "value": {"stringValue": "local"}}
            ]},
            "scopeSpans": [{
                "scope": {"name": "sts2.runtime.telemetry", "version": "1"},
                "spans": spans
            }]
        }]
    });
    let bytes = match serde_json::to_vec(&body) {
        Ok(bytes) if bytes.len() <= MAX_BODY_BYTES => bytes,
        _ => return false,
    };
    post_otlp(&bytes)
}

fn post_otlp(body: &[u8]) -> bool {
    let mut stream = match TcpStream::connect_timeout(&ENDPOINT, SOCKET_TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    if stream.set_read_timeout(Some(SOCKET_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(SOCKET_TIMEOUT)).is_err()
    {
        return false;
    }
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:14318\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        OTLP_PATH,
        body.len()
    );
    if stream.write_all(request.as_bytes()).is_err() || stream.write_all(body).is_err() {
        let _ = stream.shutdown(Shutdown::Both);
        return false;
    }
    let mut response = vec![0_u8; MAX_RESPONSE_BYTES];
    let size = match stream.read(&mut response) {
        Ok(size) => size,
        Err(_) => {
            let _ = stream.shutdown(Shutdown::Both);
            return false;
        }
    };
    let mut total = size;
    while total < MAX_RESPONSE_BYTES {
        match stream.read(&mut response[total..]) {
            Ok(0) => break,
            Ok(read) => total += read,
            Err(_) => break,
        }
    }
    let response = &response[..total];
    let first_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let accepted = first_line.starts_with(b"HTTP/1.1 2") || first_line.starts_with(b"HTTP/1.0 2");
    let partial_rejection = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .and_then(|header_end| serde_json::from_slice::<Value>(&response[header_end + 4..]).ok())
        .is_some_and(|value| contains_rejected_spans(&value));
    let _ = stream.shutdown(Shutdown::Both);
    accepted && !partial_rejection
}

fn contains_rejected_spans(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "rejectedSpans" && value.as_u64().is_some_and(|count| count > 0))
                || contains_rejected_spans(value)
        }),
        Value::Array(values) => values.iter().any(contains_rejected_spans),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn render_span(context: &TelemetryContext, event: &TelemetryEvent, sequence: u64) -> Value {
    let (kind, status_error, attrs) = event_attributes(context, event);
    let now = unix_nanos();
    let trace_id = digest("trace", &context.trace_id)[..32].to_owned();
    let root_span_id = digest("root-span", &context.trace_id)[..16].to_owned();
    let span_id = if kind == EventKind::RunStarted {
        root_span_id.clone()
    } else {
        digest(
            "span",
            &format!("{}:{sequence}:{}", context.trace_id, kind.as_str()),
        )[..16]
            .to_owned()
    };
    let attributes = attrs
        .into_iter()
        .map(|(key, value)| json!({"key": key, "value": {"stringValue": value}}))
        .collect::<Vec<_>>();
    let mut span = json!({
        "traceId": trace_id,
        "spanId": span_id,
        "name": format!("sts2.{}", kind.as_str()),
        "startTimeUnixNano": now.to_string(),
        "endTimeUnixNano": now.to_string(),
        "attributes": attributes,
        "status": {"code": if status_error {"STATUS_CODE_ERROR"} else {"STATUS_CODE_UNSET"}}
    });
    if kind != EventKind::RunStarted {
        span["parentSpanId"] = Value::String(root_span_id);
    }
    span
}

fn event_attributes(
    context: &TelemetryContext,
    event: &TelemetryEvent,
) -> (EventKind, bool, Vec<(&'static str, String)>) {
    let mut attrs = vec![
        ("sts2.event", String::new()),
        ("sts2.run_id", context.run_id.clone()),
        ("sts2.episode_id", context.episode_id.clone()),
        ("sts2.trajectory_id", context.trajectory_id.clone()),
        ("sts2.trace_id", context.trace_id.clone()),
        ("sts2.instance_id", context.instance_id.clone()),
        ("sts2.session_id", context.session_id.clone()),
        (
            "sts2.provider_revision_digest",
            context.provider_revision_digest.clone(),
        ),
        ("sts2.schema_version", context.schema_version.clone()),
        ("sts2.runtime_profile", context.runtime_profile.clone()),
    ];
    let kind = match event {
        TelemetryEvent::RunStarted => EventKind::RunStarted,
        TelemetryEvent::ModelDecision {
            model_execution_id,
            decision_kind,
            action_id_digest,
            operation_id_digest,
            confidence,
        } => {
            let kind = EventKind::ModelDecision;
            add(
                &mut attrs,
                "sts2.model_execution_id",
                model_execution_id.to_string(),
            );
            add(
                &mut attrs,
                "sts2.status",
                decision_kind_name(*decision_kind),
            );
            add_optional(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            if let Some(confidence) = confidence {
                add(&mut attrs, "sts2.confidence", confidence.to_string());
            }
            kind
        }
        TelemetryEvent::ModelFailure {
            model_execution_id,
            failure_code,
        } => {
            add(
                &mut attrs,
                "sts2.model_execution_id",
                model_execution_id.to_string(),
            );
            add(&mut attrs, "sts2.failure_code", failure_code.as_str());
            EventKind::ModelFailure
        }
        TelemetryEvent::Observation {
            source,
            generation,
            stage,
            state_id_digest,
        } => {
            add(&mut attrs, "sts2.source", source.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.state_id_digest", state_id_digest);
            EventKind::Observation
        }
        TelemetryEvent::ActionDispatch {
            operation_id_digest,
            action_id_digest,
            action_kind,
            generation,
            status,
            failure_code,
        } => {
            add(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add(&mut attrs, "sts2.action_kind", action_kind.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.status", status.as_str());
            add_optional_code(&mut attrs, "sts2.failure_code", failure_code);
            EventKind::ActionDispatch
        }
        TelemetryEvent::SettlementObservation {
            operation_id_digest,
            action_id_digest,
            from_generation,
            to_generation,
            stage,
            effect_class,
            effect_digest,
            source,
        } => {
            add(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.action_id_digest", action_id_digest);
            add(
                &mut attrs,
                "sts2.from_generation",
                from_generation.to_string(),
            );
            add(&mut attrs, "sts2.to_generation", to_generation.to_string());
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.effect_class", *effect_class);
            add(&mut attrs, "sts2.effect_digest", effect_digest);
            add(&mut attrs, "sts2.source", source.as_str());
            EventKind::SettlementObservation
        }
        TelemetryEvent::Recovery {
            kind,
            operation_id_digest,
            attempt,
            outcome,
            failure_code,
        } => {
            add(&mut attrs, "sts2.recovery_kind", kind.as_str());
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            add(&mut attrs, "sts2.recovery_attempt", attempt.to_string());
            add(&mut attrs, "sts2.status", *outcome);
            add_optional_code(&mut attrs, "sts2.failure_code", failure_code);
            EventKind::Recovery
        }
        TelemetryEvent::Failure {
            boundary,
            failure_code,
            retryable,
            operation_id_digest,
        } => {
            add(&mut attrs, "sts2.boundary", *boundary);
            add(&mut attrs, "sts2.failure_code", failure_code.as_str());
            add(&mut attrs, "sts2.retryable", bool_string(*retryable));
            add_optional(&mut attrs, "sts2.operation_id_digest", operation_id_digest);
            EventKind::Failure
        }
        TelemetryEvent::TerminalObserved {
            stage,
            outcome,
            generation,
            state_id_digest,
        } => {
            add(&mut attrs, "sts2.stage", stage.as_str());
            add(&mut attrs, "sts2.game_outcome", outcome.as_str());
            add(&mut attrs, "sts2.generation", generation.to_string());
            add(&mut attrs, "sts2.state_id_digest", state_id_digest);
            EventKind::TerminalObserved
        }
        TelemetryEvent::RunFinished {
            outcome,
            terminal_stage,
            cleanup_status,
            dropped_events,
        } => {
            add(&mut attrs, "sts2.game_outcome", outcome.as_str());
            add(&mut attrs, "sts2.stage", terminal_stage.as_str());
            add(&mut attrs, "sts2.cleanup_status", cleanup_status.as_str());
            add(
                &mut attrs,
                "sts2.dropped_events",
                dropped_events.to_string(),
            );
            add(&mut attrs, "sts2.export_status", "queued");
            EventKind::RunFinished
        }
    };
    attrs[0].1 = kind.as_str().to_owned();
    let status_error = matches!(kind, EventKind::ModelFailure | EventKind::Failure);
    (kind, status_error, attrs)
}

fn add(attrs: &mut Vec<(&'static str, String)>, key: &'static str, value: impl Into<String>) {
    attrs.push((key, value.into()));
}

fn add_optional(
    attrs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        add(attrs, key, value);
    }
}

fn add_optional_code(
    attrs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: &Option<FailureCode>,
) {
    if let Some(value) = value {
        add(attrs, key, value.as_str());
    }
}

fn decision_kind_name(kind: DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Action => "action",
        DecisionKind::Plan => "plan",
        DecisionKind::Wait => "wait",
        DecisionKind::Reobserve => "reobserve",
        DecisionKind::Recovery => "recovery",
    }
}

fn bool_string(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn effect_class(value: &str) -> &'static str {
    match value {
        "card_play" | "card_played" => "card_play",
        "end_turn" | "turn_ended" => "end_turn",
        "reward" | "reward_selected" => "reward",
        "shop_purchase" | "shop_remove" => "shop",
        "map" | "map_node_selected" => "map",
        "victory" | "defeat" => "terminal",
        _ => "other",
    }
}

fn digest(domain: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn valid_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::{
        DecisionKind, DispatchTelemetryStatus, EventKind, FailureCode, GameOutcome,
        ObservationSource, TelemetryContext, TelemetryEvent, TelemetryStage, event_attributes,
        post_otlp, render_span,
    };
    use serde_json::{Value, json};

    fn context() -> Result<TelemetryContext, String> {
        TelemetryContext::new(
            "run-test",
            "episode-test",
            "trajectory-test",
            "trace-test",
            "instance-test",
            "session-test",
            "runtime-v3-gameplay",
            "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        )
    }

    #[test]
    fn context_keeps_lineage_namespaces_distinct() {
        let duplicate = TelemetryContext::new(
            "run",
            "run",
            "trajectory",
            "trace",
            "instance",
            "session",
            "profile",
            &"a".repeat(64),
        );
        assert!(duplicate.is_err());
        let invalid_revision = TelemetryContext::new(
            "run",
            "episode",
            "trajectory",
            "trace",
            "instance",
            "session",
            "profile",
            "invalid",
        );
        assert!(invalid_revision.is_err());
    }

    #[test]
    fn rendered_decision_contains_only_allowlisted_private_safe_fields() -> Result<(), String> {
        let event = TelemetryEvent::ModelDecision {
            model_execution_id: 7,
            decision_kind: DecisionKind::Action,
            action_id_digest: Some(String::from("abc")),
            operation_id_digest: None,
            confidence: Some(90),
        };
        let body = render_span(&context()?, &event, 1).to_string();
        assert!(!body.contains("SENTINEL_PRIVATE_PROMPT"));
        assert!(!body.contains("rationale"));
        assert!(!body.contains("model_output"));
        assert!(body.contains("sts2.model_execution_id"));
        assert!(body.contains("sts2.action_id_digest"));
        assert!(body.contains("sts2.instance_id"));
        assert!(body.contains("sts2.session_id"));
        Ok(())
    }

    #[test]
    fn settlement_is_a_distinct_typed_event() -> Result<(), String> {
        let event = TelemetryEvent::SettlementObservation {
            operation_id_digest: String::from("operation-digest"),
            action_id_digest: String::from("action-digest"),
            from_generation: 4,
            to_generation: 5,
            stage: TelemetryStage::Combat,
            effect_class: "card_play",
            effect_digest: String::from("effect-digest"),
            source: ObservationSource::Transition,
        };
        let (kind, error, attrs) = event_attributes(&context()?, &event);
        assert_eq!(kind, EventKind::SettlementObservation);
        assert!(!error);
        assert!(
            attrs
                .iter()
                .any(|(key, value)| *key == "sts2.from_generation" && value == "4")
        );
        assert!(
            attrs
                .iter()
                .any(|(key, value)| *key == "sts2.to_generation" && value == "5")
        );
        Ok(())
    }

    #[test]
    fn status_and_outcome_are_finite_strings() -> Result<(), String> {
        assert_eq!(DispatchTelemetryStatus::Settled.as_str(), "settled");
        assert_eq!(
            FailureCode::ProviderUnavailable.as_str(),
            "provider_unavailable"
        );
        assert_eq!(GameOutcome::Success.as_str(), "success");
        assert_eq!(ObservationSource::Recovery.as_str(), "recovery");
        let rendered = render_span(&context()?, &TelemetryEvent::RunStarted, 2);
        assert!(
            rendered
                .get("traceId")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() == 32)
        );
        assert!(
            rendered
                .get("spanId")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() == 16)
        );
        assert!(rendered.get("parentSpanId").is_none());
        let child = render_span(
            &context()?,
            &TelemetryEvent::Observation {
                source: ObservationSource::Observe,
                generation: 1,
                stage: TelemetryStage::Setup,
                state_id_digest: String::from("state-digest"),
            },
            3,
        );
        assert_eq!(child.get("parentSpanId"), rendered.get("spanId"));
        Ok(())
    }

    #[test]
    fn collector_encoding_smoke_is_opt_in() -> Result<(), String> {
        if std::env::var("STS2_TELEMETRY_COLLECTOR_SMOKE").as_deref() != Ok("true") {
            return Ok(());
        }
        let context = context()?;
        let root = render_span(&context, &TelemetryEvent::RunStarted, 1);
        let child = render_span(
            &context,
            &TelemetryEvent::ModelDecision {
                model_execution_id: 1,
                decision_kind: DecisionKind::Action,
                action_id_digest: Some(String::from("a")),
                operation_id_digest: None,
                confidence: Some(80),
            },
            2,
        );
        let body = serde_json::to_vec(&json!({
            "resourceSpans": [{
                "resource": {"attributes": [
                    {"key": "service.name", "value": {"stringValue": "sts2-harness"}},
                    {"key": "service.version", "value": {"stringValue": "runtime-v3"}},
                    {"key": "deployment.environment", "value": {"stringValue": "local"}}
                ]},
                "scopeSpans": [{
                    "scope": {"name": "sts2.runtime.telemetry", "version": "1"},
                    "spans": [root, child]
                }]
            }]
        }))
        .map_err(|error| error.to_string())?;
        assert!(post_otlp(&body));
        Ok(())
    }
}
