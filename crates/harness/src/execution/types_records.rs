// SPDX-License-Identifier: MIT

use super::core::{Checkpoint, ExecutionFingerprint, ExecutionLineage, valid_id, valid_reference};
use super::enums::{
    AttemptKind, AttemptState, CompletionStatus, JobState, OperationState, ProviderFailureClass,
    ProviderReservationState,
};
use super::error::ExecutionStoreError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationIntent {
    pub lineage: ExecutionLineage,
    pub operation_id: String,
    pub state_id: String,
    pub generation: u64,
    pub action_id: String,
    pub payload_digest: String,
    pub input_digest: String,
}

impl OperationIntent {
    pub fn new(
        lineage: ExecutionLineage,
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
        payload_digest: impl Into<String>,
        input_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let intent = Self {
            lineage,
            operation_id: operation_id.into(),
            state_id: state_id.into(),
            generation,
            action_id: action_id.into(),
            payload_digest: payload_digest.into(),
            input_digest: input_digest.into(),
        };
        if intent.lineage.validate().is_err()
            || !valid_id(&intent.operation_id)
            || !valid_id(&intent.state_id)
            || !valid_id(&intent.action_id)
            || intent.generation > 9_007_199_254_740_991
            || !valid_reference(&intent.payload_digest)
            || !valid_reference(&intent.input_digest)
        {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        Ok(intent)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReference {
    pub lineage: ExecutionLineage,
    pub execution_id: String,
    pub input_fingerprint: String,
    pub model_revision: String,
    pub config_digest: String,
    pub result_ref: Option<String>,
    pub result_digest: Option<String>,
}

impl DecisionReference {
    pub fn new(
        lineage: ExecutionLineage,
        execution_id: impl Into<String>,
        input_fingerprint: impl Into<String>,
        model_revision: impl Into<String>,
        config_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let decision = Self {
            lineage,
            execution_id: execution_id.into(),
            input_fingerprint: input_fingerprint.into(),
            model_revision: model_revision.into(),
            config_digest: config_digest.into(),
            result_ref: None,
            result_digest: None,
        };
        if decision.lineage.validate().is_err()
            || !valid_id(&decision.execution_id)
            || !valid_reference(&decision.input_fingerprint)
            || !valid_reference(&decision.model_revision)
            || !valid_reference(&decision.config_digest)
        {
            return Err(ExecutionStoreError::InvalidDecision);
        }
        Ok(decision)
    }

    pub fn with_result(
        mut self,
        result_ref: impl Into<String>,
        result_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let result_ref = result_ref.into();
        let result_digest = result_digest.into();
        if !valid_reference(&result_ref) || !valid_reference(&result_digest) {
            return Err(ExecutionStoreError::InvalidDecision);
        }
        self.result_ref = Some(result_ref);
        self.result_digest = Some(result_digest);
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderReservation {
    pub lineage: ExecutionLineage,
    pub reservation_id: String,
    pub execution_id: String,
    pub provider_execution_id: String,
    pub reserved_units: u64,
    pub actual_units: Option<u64>,
    pub state: ProviderReservationState,
    pub failure: Option<ProviderFailureClass>,
}

impl ProviderReservation {
    pub fn new(
        lineage: ExecutionLineage,
        reservation_id: impl Into<String>,
        execution_id: impl Into<String>,
        provider_execution_id: impl Into<String>,
        reserved_units: u64,
    ) -> Result<Self, ExecutionStoreError> {
        let reservation = Self {
            lineage,
            reservation_id: reservation_id.into(),
            execution_id: execution_id.into(),
            provider_execution_id: provider_execution_id.into(),
            reserved_units,
            actual_units: None,
            state: ProviderReservationState::Reserved,
            failure: None,
        };
        if reservation.lineage.validate().is_err()
            || !valid_id(&reservation.reservation_id)
            || !valid_id(&reservation.execution_id)
            || !valid_id(&reservation.provider_execution_id)
            || reserved_units == 0
        {
            return Err(ExecutionStoreError::InvalidProviderReservation);
        }
        Ok(reservation)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if self.lineage.validate().is_err()
            || !valid_id(&self.reservation_id)
            || !valid_id(&self.execution_id)
            || !valid_id(&self.provider_execution_id)
            || self.reserved_units == 0
            || self.actual_units.is_some_and(|units| units == 0)
            || matches!(self.state, ProviderReservationState::Reserved)
                != (self.actual_units.is_none() && self.failure.is_none())
        {
            return Err(ExecutionStoreError::InvalidProviderReservation);
        }
        if matches!(self.state, ProviderReservationState::Completed)
            && (self.actual_units.is_none() || self.failure.is_some())
        {
            return Err(ExecutionStoreError::InvalidProviderReservation);
        }
        if matches!(
            self.state,
            ProviderReservationState::Failed | ProviderReservationState::Unknown
        ) && self.failure.is_none()
        {
            return Err(ExecutionStoreError::InvalidProviderReservation);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionRecord {
    pub lineage: ExecutionLineage,
    pub status: CompletionStatus,
    pub terminal_ref: String,
    pub checkpoint_sequence: u64,
    pub result_digest: String,
}

impl CompletionRecord {
    pub fn new(
        lineage: ExecutionLineage,
        status: CompletionStatus,
        terminal_ref: impl Into<String>,
        checkpoint_sequence: u64,
        result_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let completion = Self {
            lineage,
            status,
            terminal_ref: terminal_ref.into(),
            checkpoint_sequence,
            result_digest: result_digest.into(),
        };
        if completion.lineage.validate().is_err()
            || completion.checkpoint_sequence > 9_007_199_254_740_991
            || !valid_reference(&completion.terminal_ref)
            || !valid_reference(&completion.result_digest)
        {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        Ok(completion)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if self.checkpoint_sequence > 9_007_199_254_740_991
            || !valid_reference(&self.terminal_ref)
            || !valid_reference(&self.result_digest)
        {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        self.lineage.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobClaim {
    pub job_id: String,
    pub episode_id: String,
    pub claim_token: String,
    pub worker_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobClaimOutcome {
    Claimed(JobClaim),
    AlreadyClaimed(JobClaim),
    AlreadyCompleted(StoredJob),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeState {
    New,
    Ready {
        checkpoint: Option<Checkpoint>,
        pending_operations: Vec<StoredOperation>,
        pending_decisions: Vec<StoredDecision>,
    },
    Completed(CompletionRecord),
    ReconstructionRequired {
        reason: String,
    },
    InterruptedUnknown {
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEpisode {
    pub lineage: ExecutionLineage,
    pub fingerprint: ExecutionFingerprint,
    pub state: AttemptState,
    pub last_checkpoint: Option<Checkpoint>,
    pub completion: Option<CompletionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredAttempt {
    pub lineage: ExecutionLineage,
    pub kind: AttemptKind,
    pub state: AttemptState,
    pub parent_attempt_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredOperation {
    pub intent: OperationIntent,
    pub state: OperationState,
    pub result_ref: Option<String>,
    pub result_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDecision {
    pub reference: DecisionReference,
    pub completed: bool,
    pub unknown: bool,
    pub provider_reservation_id: Option<String>,
    /// Exact validated provider result bytes, when this completion was recorded by a
    /// result-aware writer. Historical metadata-only completions remain `None` and are not
    /// eligible for replay.
    pub result_payload: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredJob {
    pub job_id: String,
    pub episode_id: String,
    pub payload_digest: String,
    pub state: JobState,
    pub claim: Option<JobClaim>,
    pub result_ref: Option<String>,
}
