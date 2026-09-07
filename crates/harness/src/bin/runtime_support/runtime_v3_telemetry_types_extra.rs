// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameOutcome {
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
pub enum CleanupStatus {
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
pub enum ObservationSource {
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
pub enum DispatchTelemetryStatus {
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
    ExportStatus,
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
            Self::ExportStatus => "export_status",
        }
    }
}
