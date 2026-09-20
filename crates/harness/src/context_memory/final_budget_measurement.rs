// SPDX-License-Identifier: MIT

// Qualified token measurements for the final prepared-input budget.
//
// A byte ceiling is never a token measurement. Every quantity carries its provenance and scope so
// that an absent measurement, a declared heuristic, a bound local tokenizer and a provider-reported
// total stay distinguishable, and no constructor accepts a byte count as a token count. The record
// is harness-internal: no consumer pin adopts it, so it is deliberately absent from `contracts/`.

pub const PREPARED_INPUT_BUDGET_SCHEMA: &str = "ascension.harness.prepared-input-budget.v1";
/// Harness runtime guard for the output capacity reserved beside one prepared input.
pub const MAX_PREPARED_OUTPUT_RESERVE_BYTES: usize = 8 * 1024;
/// Admitted field names read from the published effective-limit record.
const WHOLE_INPUT_LIMIT_FIELD: &str = "max_job_input_bytes";
const OPTIONAL_BUDGET_LIMIT_FIELD: &str = "optional_byte_budget";

/// Provenance of one token quantity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenProvenance {
    /// No tokenizer or provider measurement is bound; the quantity is not a token count.
    Unavailable,
    /// A declared approximation. It is preserved and labelled, never promoted to a guarantee.
    Heuristic,
    /// A bound local tokenizer measured the quantity.
    LocalTokenizer,
    /// The provider reported the quantity for a completed turn.
    ProviderReported,
}

impl TokenProvenance {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Heuristic => "heuristic",
            Self::LocalTokenizer => "local_tokenizer",
            Self::ProviderReported => "provider_reported",
        }
    }

    /// Only a bound tokenizer or the provider itself establishes a measured quantity.
    #[must_use]
    pub fn is_measured(self) -> bool {
        matches!(self, Self::LocalTokenizer | Self::ProviderReported)
    }
}

/// Which quantity a measurement covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementScope {
    /// One component of the final input.
    Component,
    /// The whole prepared input, including framing, tool/schema material and protected entries.
    PreparedInput,
    /// One provider turn.
    ProviderTurn,
    /// A running total across turns.
    Cumulative,
}

impl MeasurementScope {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::PreparedInput => "prepared_input",
            Self::ProviderTurn => "provider_turn",
            Self::Cumulative => "cumulative",
        }
    }
}

/// One token quantity with its provenance, scope and bound method.
///
/// The quantity is `None` exactly when nothing is bound, so a caller cannot read a byte count as a
/// token count through this type.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenMeasurement {
    pub provenance: TokenProvenance,
    pub scope: MeasurementScope,
    /// `None` exactly when the provenance is [`TokenProvenance::Unavailable`].
    pub tokens: Option<u64>,
    /// Bound measurement adapter identity, or `none` when nothing is bound.
    pub method: String,
}

impl TokenMeasurement {
    /// An explicitly absent measurement. Absent is not zero.
    #[must_use]
    pub fn unavailable(scope: MeasurementScope) -> Self {
        Self {
            provenance: TokenProvenance::Unavailable,
            scope,
            tokens: None,
            method: "none".to_owned(),
        }
    }

    /// A declared approximation labelled with the method that produced it.
    #[must_use]
    pub fn heuristic(scope: MeasurementScope, tokens: u64, method: impl Into<String>) -> Self {
        Self::measured(TokenProvenance::Heuristic, scope, tokens, method)
    }

    /// A quantity measured by the bound local tokenizer.
    #[must_use]
    pub fn local_tokenizer(scope: MeasurementScope, tokens: u64, method: impl Into<String>) -> Self {
        Self::measured(TokenProvenance::LocalTokenizer, scope, tokens, method)
    }

    /// A quantity reported by the provider for a completed turn.
    #[must_use]
    pub fn provider_reported(
        scope: MeasurementScope,
        tokens: u64,
        method: impl Into<String>,
    ) -> Self {
        Self::measured(TokenProvenance::ProviderReported, scope, tokens, method)
    }

    fn measured(
        provenance: TokenProvenance,
        scope: MeasurementScope,
        tokens: u64,
        method: impl Into<String>,
    ) -> Self {
        Self {
            provenance,
            scope,
            tokens: Some(tokens),
            method: method.into(),
        }
    }

    /// The measured or heuristic quantity; `None` when nothing is bound.
    #[must_use]
    pub fn tokens(&self) -> Option<u64> {
        self.tokens
    }

    /// Whether this quantity is an exact measurement for its scope.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.provenance.is_measured() && self.tokens.is_some()
    }

    #[must_use]
    pub fn qualifier(&self) -> &'static str {
        self.provenance.code()
    }

    /// Qualification text for a report or manifest; it never states a byte count as tokens.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.tokens {
            Some(tokens) => format!(
                "{} {} tokens={tokens} method={}",
                self.qualifier(),
                self.scope.code(),
                self.method
            ),
            None => format!("{} {} method=none", self.qualifier(), self.scope.code()),
        }
    }

    fn validate(&self) -> Result<(), PreparedBudgetError> {
        if !valid_id(&self.method) {
            return Err(PreparedBudgetError::InvalidMeasurement("method"));
        }
        match (self.provenance, self.tokens) {
            (TokenProvenance::Unavailable, None) => Ok(()),
            (TokenProvenance::Unavailable, Some(_)) => {
                Err(PreparedBudgetError::InvalidMeasurement(
                    "unavailable_provenance_with_tokens",
                ))
            }
            (_, None) => Err(PreparedBudgetError::InvalidMeasurement("tokens")),
            (_, Some(0)) => Err(PreparedBudgetError::InvalidMeasurement("tokens")),
            (_, Some(_)) => Ok(()),
        }
    }
}

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
    /// Input plus the output reserve exceed the combined profile window.
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
