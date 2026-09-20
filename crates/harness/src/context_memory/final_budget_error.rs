// SPDX-License-Identifier: MIT

// Typed rejection of a prepared input before any provider reservation is written.
//
// The rejections are separated from the measurement vocabulary they qualify so the measurement
// invariant and the refusal vocabulary are each reviewable as one boundary.

/// One axis whose change invalidates a previously approved preparation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedInputDrift {
    OwnerRevision,
    Profile,
    Model,
    Tokenizer,
    Adapter,
    EffectiveLimit,
}

impl PreparedInputDrift {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::OwnerRevision => "owner_revision",
            Self::Profile => "profile",
            Self::Model => "model",
            Self::Tokenizer => "tokenizer",
            Self::Adapter => "adapter",
            Self::EffectiveLimit => "effective_limit",
        }
    }
}

/// Typed rejection of a prepared input before any provider reservation is written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedBudgetError {
    /// The request is structurally invalid for the named field.
    InvalidRequest(&'static str),
    /// The named measurement field is invalid.
    InvalidMeasurement(&'static str),
    /// A published profile ceiling rejects a schema-valid value.
    ProfileInadmissible {
        field: String,
        requested: u64,
        reason: crate::effective_limits::UnavailableReason,
    },
    /// Framing, tool/schema, mandatory or pinned bytes cannot fit the admission window.
    ProtectedOverflow {
        field: &'static str,
        requested: usize,
        effective: usize,
        overflow_bytes: usize,
    },
    /// The reserved output capacity cannot fit beside the input bound.
    OutputReserveOverflow {
        requested: usize,
        effective: usize,
    },
    /// Input plus the output reserve exceed the combined whole-input bound or profile window.
    CombinedWindowOverflow {
        input_bytes: usize,
        output_reserve_bytes: usize,
        effective: usize,
    },
    /// A recorded profile/model/tokenizer/adapter/limit axis changed.
    ApprovalInvalidated(Vec<PreparedInputDrift>),
    /// The provider reservation ledger refused the capacity reservation.
    ReservationRejected(MemoryError),
    /// A measurement was attached with a scope the budget does not own.
    MeasurementScope(&'static str),
}

impl std::fmt::Display for PreparedBudgetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(field) => {
                write!(formatter, "prepared input field {field} is invalid")
            }
            Self::InvalidMeasurement(field) => write!(
                formatter,
                "prepared input token measurement {field} is invalid; a byte ceiling is never a token count"
            ),
            Self::ProfileInadmissible {
                field,
                requested,
                reason,
            } => write!(
                formatter,
                "prepared input rejects before any provider reservation: {field} requests {requested} and the selected profile reports {}",
                reason.code()
            ),
            Self::ProtectedOverflow {
                field,
                requested,
                effective,
                overflow_bytes,
            } => write!(
                formatter,
                "prepared input rejects before any provider reservation: protected framing, tool/schema, mandatory and pinned bytes need {requested} against {field} {effective}, over by {overflow_bytes}; reduce protected content or raise the admitted profile ceiling"
            ),
            Self::OutputReserveOverflow {
                requested,
                effective,
            } => write!(
                formatter,
                "prepared input rejects before any provider reservation: the output reserve of {requested} exceeds {effective}"
            ),
            Self::CombinedWindowOverflow {
                input_bytes,
                output_reserve_bytes,
                effective,
            } => write!(
                formatter,
                "prepared input rejects before any provider reservation: input {input_bytes} plus output reserve {output_reserve_bytes} exceeds the combined window {effective}"
            ),
            Self::ApprovalInvalidated(drift) => {
                let axes = drift
                    .iter()
                    .map(|axis| axis.code())
                    .collect::<Vec<_>>()
                    .join(",");
                write!(
                    formatter,
                    "prepared input approval is invalidated by changed axes: {axes}"
                )
            }
            Self::ReservationRejected(error) => {
                write!(formatter, "prepared input reservation was refused: {error}")
            }
            Self::MeasurementScope(scope) => write!(
                formatter,
                "prepared input does not own a {scope} scoped measurement"
            ),
        }
    }
}

impl std::error::Error for PreparedBudgetError {}
