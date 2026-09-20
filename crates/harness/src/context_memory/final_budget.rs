// SPDX-License-Identifier: MIT

// Final prepared-input request and recorded budget.
//
// `SelectionManifest` bounds the memory half of a prepared input. The bytes a provider actually
// receives also carry framing, tool/schema material, mandatory inputs and pinned entries, and the
// provider turn reserves output capacity separately. This record owns that final arithmetic so the
// protected components, the admitted whole-input bound and the separate output reserve are visible
// together, with the token quantity staying qualified by its provenance.

/// Identity axes a prepared input is bound to. A change on any axis invalidates a preview.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedInputPins {
    pub owner_revision: String,
    pub profile_id: String,
    pub model_id: String,
    pub tokenizer_id: String,
    pub adapter_id: String,
    /// Digest of the effective-limit record this preparation was admitted against.
    pub effective_limit_digest: String,
}

impl PreparedInputPins {
    fn validate(&self) -> Result<(), PreparedBudgetError> {
        for (field, value) in [
            ("owner_revision", &self.owner_revision),
            ("profile_id", &self.profile_id),
            ("model_id", &self.model_id),
            ("tokenizer_id", &self.tokenizer_id),
            ("adapter_id", &self.adapter_id),
        ] {
            if !valid_id(value) {
                return Err(PreparedBudgetError::InvalidRequest(field));
            }
        }
        if !valid_digest(&self.effective_limit_digest) {
            return Err(PreparedBudgetError::InvalidRequest("effective_limit_digest"));
        }
        Ok(())
    }

    /// Every axis whose change invalidates a prior approval, reported in a fixed order.
    #[must_use]
    pub fn drift(&self, current: &Self) -> Vec<PreparedInputDrift> {
        let mut drift = Vec::new();
        if self.owner_revision != current.owner_revision {
            drift.push(PreparedInputDrift::OwnerRevision);
        }
        if self.profile_id != current.profile_id {
            drift.push(PreparedInputDrift::Profile);
        }
        if self.model_id != current.model_id {
            drift.push(PreparedInputDrift::Model);
        }
        if self.tokenizer_id != current.tokenizer_id {
            drift.push(PreparedInputDrift::Tokenizer);
        }
        if self.adapter_id != current.adapter_id {
            drift.push(PreparedInputDrift::Adapter);
        }
        if self.effective_limit_digest != current.effective_limit_digest {
            drift.push(PreparedInputDrift::EffectiveLimit);
        }
        drift
    }
}

/// Executable bounds for one prepared input.
///
/// `whole_input_byte_bound` admits the final input; `output_reserve_bytes` is reserved beside it and
/// is never folded into the input bytes. `combined_window_bytes` is the profile's combined window
/// when one is published.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedInputLimits {
    pub whole_input_byte_bound: usize,
    pub optional_byte_budget: usize,
    pub output_reserve_bytes: usize,
    pub combined_window_bytes: Option<usize>,
}

impl PreparedInputLimits {
    fn validate(&self) -> Result<(), PreparedBudgetError> {
        if self.whole_input_byte_bound == 0 || self.whole_input_byte_bound > MAX_JOB_INPUT_BYTES {
            return Err(PreparedBudgetError::InvalidRequest("whole_input_byte_bound"));
        }
        if self.optional_byte_budget == 0
            || self.optional_byte_budget > MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES
        {
            return Err(PreparedBudgetError::InvalidRequest("optional_byte_budget"));
        }
        if self.output_reserve_bytes == 0
            || self.output_reserve_bytes > MAX_PREPARED_OUTPUT_RESERVE_BYTES
        {
            return Err(PreparedBudgetError::InvalidRequest("output_reserve_bytes"));
        }
        if self.combined_window_bytes == Some(0) {
            return Err(PreparedBudgetError::InvalidRequest("combined_window_bytes"));
        }
        Ok(())
    }
}

/// One included component of the final input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedComponentRef {
    pub entry_id: String,
    pub sha256: String,
    pub bytes: usize,
}

/// A protected entry. It is never evicted to fit a bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedInput {
    pub entry_id: String,
    pub sha256: String,
    pub content: Vec<u8>,
}

/// An evictable entry. Admitted optional content is selected in a deterministic order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalInput {
    pub entry_id: String,
    pub sha256: String,
    pub content: Vec<u8>,
}

impl PinnedInput {
    fn validate(&self) -> Result<(), PreparedBudgetError> {
        validate_component(&self.entry_id, &self.sha256, &self.content, "pinned")
    }

    #[must_use]
    pub fn reference(&self) -> PreparedComponentRef {
        PreparedComponentRef {
            entry_id: self.entry_id.clone(),
            sha256: self.sha256.clone(),
            bytes: self.content.len(),
        }
    }
}

impl OptionalInput {
    fn validate(&self) -> Result<(), PreparedBudgetError> {
        validate_component(&self.entry_id, &self.sha256, &self.content, "optional")
    }

    #[must_use]
    pub fn reference(&self) -> PreparedComponentRef {
        PreparedComponentRef {
            entry_id: self.entry_id.clone(),
            sha256: self.sha256.clone(),
            bytes: self.content.len(),
        }
    }
}

fn validate_component(
    entry_id: &str,
    sha256: &str,
    content: &[u8],
    field: &'static str,
) -> Result<(), PreparedBudgetError> {
    if !valid_id(entry_id) || !valid_digest(sha256) || content.is_empty() {
        return Err(PreparedBudgetError::InvalidRequest(field));
    }
    Ok(())
}

/// Everything the owner must supply for one final prepared input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedInputRequest {
    pub request_id: String,
    pub pins: PreparedInputPins,
    pub limits: PreparedInputLimits,
    /// Final wrapper bytes, including the closing application framing.
    pub framing: Vec<u8>,
    /// Tool and response-schema material sent beside the input.
    pub tool_schema: Vec<u8>,
    /// Inputs that are required for the invocation.
    pub mandatory: Vec<u8>,
    pub pinned: Vec<PinnedInput>,
    pub optional: Vec<OptionalInput>,
    /// The whole prepared-input measurement, or an explicit absence.
    pub measurement: TokenMeasurement,
}

impl PreparedInputRequest {
    /// Structural validation only. Profile admission happens in `prepare`.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError`] when an identifier, digest, bound, component or measurement
    /// is invalid, or when the request repeats an entry id.
    pub fn validate(&self) -> Result<(), PreparedBudgetError> {
        if !valid_id(&self.request_id) {
            return Err(PreparedBudgetError::InvalidRequest("request_id"));
        }
        self.pins.validate()?;
        self.limits.validate()?;
        self.measurement.validate()?;
        if self.measurement.scope != MeasurementScope::PreparedInput {
            return Err(PreparedBudgetError::MeasurementScope(
                self.measurement.scope.code(),
            ));
        }
        if self.pinned.len() > MAX_SELECTED || self.optional.len() > MAX_CANDIDATES {
            return Err(PreparedBudgetError::InvalidRequest("component_count"));
        }
        let mut ids = BTreeSet::new();
        for pin in &self.pinned {
            pin.validate()?;
            if !ids.insert(pin.entry_id.as_str()) {
                return Err(PreparedBudgetError::InvalidRequest("duplicate_entry_id"));
            }
        }
        for entry in &self.optional {
            entry.validate()?;
            if !ids.insert(entry.entry_id.as_str()) {
                return Err(PreparedBudgetError::InvalidRequest("duplicate_entry_id"));
            }
        }
        if self.protected_bytes() == 0 {
            return Err(PreparedBudgetError::InvalidRequest("empty_input"));
        }
        Ok(())
    }

    /// Framing, tool/schema, mandatory and pinned bytes; the components that cannot be evicted.
    #[must_use]
    pub fn protected_bytes(&self) -> usize {
        let pinned = self
            .pinned
            .iter()
            .fold(0usize, |total, pin| total.saturating_add(pin.content.len()));
        self.framing
            .len()
            .saturating_add(self.tool_schema.len())
            .saturating_add(self.mandatory.len())
            .saturating_add(pinned)
    }
}

/// The recorded outcome of preparing one final input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedInputBudget {
    pub schema: String,
    pub request_id: String,
    pub pins: PreparedInputPins,
    pub limits: PreparedInputLimits,
    pub framing_bytes: usize,
    pub tool_schema_bytes: usize,
    pub mandatory_bytes: usize,
    pub pinned: Vec<PreparedComponentRef>,
    pub selected_optional: Vec<PreparedComponentRef>,
    pub exclusions: Vec<ExclusionReason>,
    pub pinned_bytes: usize,
    pub optional_rendered_bytes: usize,
    /// The admitted final input length, excluding the output reserve.
    pub input_bytes: usize,
    /// The output capacity reserved beside this input.
    pub output_reserve_bytes: usize,
    pub whole_bytes_including_reserve: usize,
    pub measurement: TokenMeasurement,
    /// A later provider-reported turn quantity, attached without changing the prepared bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_report: Option<TokenMeasurement>,
    /// `bounded_unknown_tokens`, `bounded_heuristic_tokens` or `measured_within_bound`.
    pub budget_status: String,
    pub prepared_input_sha256: String,
}

impl PreparedInputBudget {
    /// The recorded profile/model/tokenizer/adapter/limit drift, in a fixed order.
    #[must_use]
    pub fn drift(&self, current: &PreparedInputPins) -> Vec<PreparedInputDrift> {
        self.pins.drift(current)
    }

    /// Revalidate a recorded budget against the current profile identities.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError::ApprovalInvalidated`] listing every changed axis.
    pub fn revalidate(&self, current: &PreparedInputPins) -> Result<(), PreparedBudgetError> {
        let drift = self.drift(current);
        if drift.is_empty() {
            return Ok(());
        }
        Err(PreparedBudgetError::ApprovalInvalidated(drift))
    }

    /// Attach a provider-reported turn measurement without changing the prepared bounds.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError`] unless the report is a measured provider-turn quantity.
    pub fn with_provider_report(
        &self,
        report: TokenMeasurement,
    ) -> Result<Self, PreparedBudgetError> {
        if report.scope != MeasurementScope::ProviderTurn
            || report.provenance != TokenProvenance::ProviderReported
        {
            return Err(PreparedBudgetError::MeasurementScope(report.scope.code()));
        }
        report.validate()?;
        let mut updated = self.clone();
        updated.provider_report = Some(report);
        Ok(updated)
    }

    /// The qualified token quantity recorded for the prepared input.
    #[must_use]
    pub fn tokens(&self) -> Option<u64> {
        self.measurement.tokens()
    }
}
