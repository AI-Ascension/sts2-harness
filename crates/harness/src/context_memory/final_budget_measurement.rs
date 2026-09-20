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
///
/// The fields are private and every read carries its provenance, so the invariant is enforced
/// wherever the quantity is observed. Both construction paths enforce it: the validated
/// constructors, and deserialization, which re-validates the record before a value exists. A crafted
/// record that claims `Unavailable` while carrying a quantity, or a quantity with no provenance to
/// label it, is rejected rather than read back as a measurement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TokenMeasurement {
    provenance: TokenProvenance,
    scope: MeasurementScope,
    /// `None` exactly when the provenance is [`TokenProvenance::Unavailable`].
    tokens: Option<u64>,
    /// Bound measurement adapter identity, or `none` when nothing is bound.
    method: String,
}

/// The unvalidated wire shape of one measurement record.
///
/// It exists only inside [`TokenMeasurement::deserialize`]: a record becomes a
/// [`TokenMeasurement`] only after the same validation every constructor applies.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenMeasurementRecord {
    provenance: TokenProvenance,
    scope: MeasurementScope,
    tokens: Option<u64>,
    method: String,
}

impl TryFrom<TokenMeasurementRecord> for TokenMeasurement {
    type Error = PreparedBudgetError;

    fn try_from(record: TokenMeasurementRecord) -> Result<Self, Self::Error> {
        let measurement = Self {
            provenance: record.provenance,
            scope: record.scope,
            tokens: record.tokens,
            method: record.method,
        };
        measurement.validate()?;
        Ok(measurement)
    }
}

impl<'de> Deserialize<'de> for TokenMeasurement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::try_from(TokenMeasurementRecord::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
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

    /// The provenance that qualifies the quantity.
    #[must_use]
    pub fn provenance(&self) -> TokenProvenance {
        self.provenance
    }

    /// The quantity this measurement covers.
    #[must_use]
    pub fn scope(&self) -> MeasurementScope {
        self.scope
    }

    /// The bound measurement adapter identity, or `none` when nothing is bound.
    #[must_use]
    pub fn method(&self) -> &str {
        self.method.as_str()
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
