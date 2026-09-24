// SPDX-License-Identifier: MIT

// Admission, deterministic eviction and provider reservation for the final prepared input.
//
// Every bound is checked before the reservation ledger is written, so protected overflow yields a
// typed rejection with zero provider reservations rather than a truncated or summarized input.

/// How profile admission is established for one preparation.
#[derive(Clone, Copy, Debug)]
pub enum PreparedInputAdmission<'a> {
    /// Admit against the trusted capability descriptor derived by the caller.
    Capability(&'a MemoryCapabilities),
    /// Admit against a published record authenticated by the trusted capability descriptor.
    Authenticated {
        capabilities: &'a MemoryCapabilities,
        record: &'a crate::effective_limits::EffectiveLimitRecord,
    },
    /// Structural bounds only. The caller owns profile admission and no profile claim is made.
    Unadmitted,
}

/// The whole-input bound and the separate output reserve for one assembled input.
///
/// [`PreparedInputRequest`] describes a request whose optional content may still be evicted. This is
/// the same arithmetic for the other shape: the exact bytes already exist, so an overflow is the
/// same refusal rather than an eviction. A served caller that has already assembled provider bytes
/// admits them through this type instead of re-deriving the subtraction beside this library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssembledInputBound {
    pub whole_input_byte_bound: usize,
    pub output_reserve_bytes: usize,
}

impl AssembledInputBound {
    /// Validate and bind one assembled-input admission.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError::InvalidRequest`] naming the offending field when the bound or
    /// the reserve is zero, or the reserve exceeds [`MAX_PREPARED_OUTPUT_RESERVE_BYTES`]. A bound is
    /// never unlimited and a reserve is never absent: both are explicit values.
    ///
    /// The bound's own surface ceiling is validated where the value is authored, not here: the
    /// memory surface bounds `whole_input_byte_bound` by [`MAX_JOB_INPUT_BYTES`] in
    /// `PreparedInputLimits::validate`, and the context-owner surface bounds
    /// `max_context_bytes` in `validate_limits`. Applying either constant here would reject the
    /// other surface's legal value.
    pub fn new(
        whole_input_byte_bound: usize,
        output_reserve_bytes: usize,
    ) -> Result<Self, PreparedBudgetError> {
        if whole_input_byte_bound == 0 {
            return Err(PreparedBudgetError::InvalidRequest(
                "whole_input_byte_bound",
            ));
        }
        if output_reserve_bytes == 0 || output_reserve_bytes > MAX_PREPARED_OUTPUT_RESERVE_BYTES {
            return Err(PreparedBudgetError::InvalidRequest("output_reserve_bytes"));
        }
        Ok(Self {
            whole_input_byte_bound,
            output_reserve_bytes,
        })
    }

    /// The input headroom the bound leaves once the reserve is set aside.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError::OutputReserveOverflow`] when the reserve cannot fit inside
    /// the whole-input bound.
    pub fn input_headroom(&self) -> Result<usize, PreparedBudgetError> {
        self.whole_input_byte_bound
            .checked_sub(self.output_reserve_bytes)
            .ok_or(PreparedBudgetError::OutputReserveOverflow {
                requested: self.output_reserve_bytes,
                effective: self.whole_input_byte_bound,
            })
    }

    /// Admit an assembled input length, returning the whole bytes including the reserve.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError::OutputReserveOverflow`] when the reserve cannot fit inside
    /// the bound, or [`PreparedBudgetError::CombinedWindowOverflow`] when the assembled input plus
    /// its reserve exceeds the bound.
    pub fn admit(&self, input_bytes: usize) -> Result<usize, PreparedBudgetError> {
        let headroom = self.input_headroom()?;
        if input_bytes > headroom {
            return Err(PreparedBudgetError::CombinedWindowOverflow {
                input_bytes,
                output_reserve_bytes: self.output_reserve_bytes,
                effective: self.whole_input_byte_bound,
            });
        }
        Ok(input_bytes.saturating_add(self.output_reserve_bytes))
    }
}

impl PreparedInputRequest {
    /// Prepare the final application input, evict optional content deterministically, and reserve
    /// provider capacity only after every bound is satisfied.
    ///
    /// The reservation is written last: a rejection means no provider reservation exists for this
    /// request, so no provider call can be dispatched from it.
    ///
    /// # Errors
    ///
    /// Returns [`PreparedBudgetError`] when the request is invalid, profile admission rejects a
    /// value, protected framing/tool-schema/mandatory/pinned bytes overflow, the output reserve or
    /// combined window is exceeded, or the ledger refuses the reservation.
    pub fn prepare(
        &self,
        admission: PreparedInputAdmission<'_>,
        ledger: &mut MemoryBudgetLedger,
        reservation_id: impl Into<String>,
    ) -> Result<PreparedInputBudget, PreparedBudgetError> {
        self.validate()?;
        self.admit(admission)?;
        let reserve = self.limits.output_reserve_bytes;
        let input_headroom = AssembledInputBound {
            whole_input_byte_bound: self.limits.whole_input_byte_bound,
            output_reserve_bytes: reserve,
        }
        .input_headroom()?;
        let protected = self.protected_bytes();
        if protected > input_headroom {
            return Err(PreparedBudgetError::ProtectedOverflow {
                field: WHOLE_INPUT_LIMIT_FIELD,
                requested: protected.saturating_add(reserve),
                effective: self.limits.whole_input_byte_bound,
                overflow_bytes: protected.saturating_sub(input_headroom),
            });
        }
        let (selected, exclusions, optional_bytes) = self.evict(protected, input_headroom);
        let input_bytes = protected.saturating_add(optional_bytes);
        if let Some(window) = self.limits.combined_window_bytes {
            let combined = input_bytes.saturating_add(reserve);
            if combined > window {
                return Err(PreparedBudgetError::CombinedWindowOverflow {
                    input_bytes,
                    output_reserve_bytes: reserve,
                    effective: window,
                });
            }
        }
        let reservation_id = reservation_id.into();
        ledger
            .reserve(reservation_id.as_str(), input_bytes, reserve)
            .map_err(PreparedBudgetError::ReservationRejected)?;
        Ok(self.record(input_bytes, optional_bytes, selected, exclusions))
    }

    fn admit(&self, admission: PreparedInputAdmission<'_>) -> Result<(), PreparedBudgetError> {
        let (capabilities, record) = match admission {
            PreparedInputAdmission::Unadmitted => return Ok(()),
            PreparedInputAdmission::Capability(capabilities) => (capabilities, None),
            PreparedInputAdmission::Authenticated {
                capabilities,
                record,
            } => (capabilities, Some(record)),
        };
        for (field, requested) in [
            (
                WHOLE_INPUT_LIMIT_FIELD,
                self.limits.whole_input_byte_bound as u64,
            ),
            (
                OPTIONAL_BUDGET_LIMIT_FIELD,
                self.limits.optional_byte_budget as u64,
            ),
        ] {
            let outcome = match record {
                Some(record) => capabilities.admit_authorized_record(record, field, requested),
                None => capabilities.admit_policy_value(field, requested),
            };
            outcome.map_err(|reason| PreparedBudgetError::ProfileInadmissible {
                field: field.to_owned(),
                requested,
                reason,
            })?;
        }
        Ok(())
    }

    /// Admit optional content in a stable order bounded by both the optional budget and the headroom
    /// left by the protected components.
    fn evict(
        &self,
        protected: usize,
        input_headroom: usize,
    ) -> (Vec<PreparedComponentRef>, Vec<ExclusionReason>, usize) {
        let available = input_headroom.saturating_sub(protected);
        let optional_limit = self.limits.optional_byte_budget.min(available);
        let mut ordered = self.optional.iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            left.entry_id
                .cmp(&right.entry_id)
                .then_with(|| left.sha256.cmp(&right.sha256))
        });
        let mut selected = Vec::new();
        let mut exclusions = Vec::new();
        let mut used = 0usize;
        for entry in ordered {
            if used.saturating_add(entry.content.len()) > optional_limit {
                exclusions.push(ExclusionReason {
                    entry_id: entry.entry_id.clone(),
                    reason: if optional_limit < self.limits.optional_byte_budget {
                        "whole_input_budget".to_owned()
                    } else {
                        "optional_byte_budget".to_owned()
                    },
                });
                continue;
            }
            used = used.saturating_add(entry.content.len());
            selected.push(entry.reference());
        }
        (selected, exclusions, used)
    }

    fn record(
        &self,
        input_bytes: usize,
        optional_bytes: usize,
        selected: Vec<PreparedComponentRef>,
        exclusions: Vec<ExclusionReason>,
    ) -> PreparedInputBudget {
        let pinned = self.pinned.iter().map(PinnedInput::reference).collect::<Vec<_>>();
        let pinned_bytes = pinned
            .iter()
            .fold(0usize, |total, pin| total.saturating_add(pin.bytes));
        PreparedInputBudget {
            schema: PREPARED_INPUT_BUDGET_SCHEMA.to_owned(),
            request_id: self.request_id.clone(),
            pins: self.pins.clone(),
            limits: self.limits,
            framing_bytes: self.framing.len(),
            tool_schema_bytes: self.tool_schema.len(),
            mandatory_bytes: self.mandatory.len(),
            pinned,
            selected_optional: selected,
            exclusions,
            pinned_bytes,
            optional_rendered_bytes: optional_bytes,
            input_bytes,
            output_reserve_bytes: self.limits.output_reserve_bytes,
            whole_bytes_including_reserve: input_bytes
                .saturating_add(self.limits.output_reserve_bytes),
            measurement: self.measurement.clone(),
            provider_report: None,
            budget_status: budget_status(&self.measurement).to_owned(),
            prepared_input_sha256: prepared_input_digest(self, optional_bytes),
        }
    }
}

fn budget_status(measurement: &TokenMeasurement) -> &'static str {
    match measurement.provenance() {
        TokenProvenance::Unavailable => "bounded_unknown_tokens",
        TokenProvenance::Heuristic => "bounded_heuristic_tokens",
        TokenProvenance::LocalTokenizer | TokenProvenance::ProviderReported => {
            "measured_within_bound"
        }
    }
}

/// Digest of the exact admitted bytes: framing, tool/schema material, mandatory inputs, every pinned
/// entry and the selected optional entries in their stable order.
fn prepared_input_digest(request: &PreparedInputRequest, optional_bytes: usize) -> String {
    let mut rendered = Vec::new();
    rendered.extend_from_slice(b"prepared-input-v1\n");
    rendered.extend_from_slice(&request.framing);
    rendered.extend_from_slice(&request.tool_schema);
    rendered.extend_from_slice(&request.mandatory);
    let mut pinned = request.pinned.iter().collect::<Vec<_>>();
    pinned.sort_by(|left, right| {
        left.entry_id
            .cmp(&right.entry_id)
            .then_with(|| left.sha256.cmp(&right.sha256))
    });
    for pin in pinned {
        rendered.extend_from_slice(pin.content.as_slice());
    }
    let mut optional = request.optional.iter().collect::<Vec<_>>();
    optional.sort_by(|left, right| {
        left.entry_id
            .cmp(&right.entry_id)
            .then_with(|| left.sha256.cmp(&right.sha256))
    });
    let mut admitted = 0usize;
    for entry in optional {
        if admitted.saturating_add(entry.content.len()) > optional_bytes {
            continue;
        }
        admitted = admitted.saturating_add(entry.content.len());
        rendered.extend_from_slice(entry.content.as_slice());
    }
    sha256_hex(rendered)
}
