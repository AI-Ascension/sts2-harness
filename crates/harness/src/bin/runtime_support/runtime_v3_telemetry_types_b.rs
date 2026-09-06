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
