// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptKind {
    Initial,
    InPlaceContinuation,
    Reconstruction,
    InterruptedUnknown,
}

impl AttemptKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::InPlaceContinuation => "in_place_continuation",
            Self::Reconstruction => "reconstruction",
            Self::InterruptedUnknown => "interrupted_unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "initial" => Self::Initial,
            "in_place_continuation" => Self::InPlaceContinuation,
            "reconstruction" => Self::Reconstruction,
            "interrupted_unknown" => Self::InterruptedUnknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState {
    Active,
    Completed,
    Failed,
    InterruptedUnknown,
    Quarantined,
}

impl AttemptState {
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "interrupted_unknown" => Self::InterruptedUnknown,
            "quarantined" => Self::Quarantined,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationState {
    IntentRecorded,
    MayHaveBeenDispatched,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Reconciled,
}

impl OperationState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IntentRecorded => "intent_recorded",
            Self::MayHaveBeenDispatched => "may_have_been_dispatched",
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::Reconciled => "reconciled",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "intent_recorded" => Self::IntentRecorded,
            "may_have_been_dispatched" => Self::MayHaveBeenDispatched,
            "accepted" => Self::Accepted,
            "settled" => Self::Settled,
            "rejected" => Self::Rejected,
            "unknown" => Self::Unknown,
            "reconciled" => Self::Reconciled,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn is_unresolved(self) -> bool {
        matches!(
            self,
            Self::IntentRecorded | Self::MayHaveBeenDispatched | Self::Accepted | Self::Unknown
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFailureClass {
    Authentication,
    Quota,
    Outage,
    Timeout,
    IncompatibleOutput,
    Cancelled,
}

impl ProviderFailureClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Quota => "quota",
            Self::Outage => "outage",
            Self::Timeout => "timeout",
            Self::IncompatibleOutput => "incompatible_output",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderReservationState {
    Reserved,
    Completed,
    Unknown,
    Failed,
}

impl ProviderReservationState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Completed => "completed",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "reserved" => Self::Reserved,
            "completed" => Self::Completed,
            "unknown" => Self::Unknown,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDisposition {
    InPlaceContinuation,
    Reconstruction,
    InterruptedUnknown,
}

impl RecoveryDisposition {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InPlaceContinuation => "in_place_continuation",
            Self::Reconstruction => "reconstruction",
            Self::InterruptedUnknown => "interrupted_unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionStatus {
    Completed,
    Failed,
    Quarantined,
}

impl CompletionStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "quarantined" => Self::Quarantined,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    Admitted,
    Claimed,
    Completed,
    Failed,
}

impl JobState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Claimed => "claimed",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "admitted" => Self::Admitted,
            "claimed" => Self::Claimed,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}
