// SPDX-License-Identifier: MIT

use crate::worker_handoff::{TerminalCompletion, TerminalRecord, TerminalStatus, WorkerRequest};
use serde_json::{Value, json};

use super::super::core::valid_reference;
use super::super::enums::CompletionStatus;
use super::super::error::ExecutionStoreError;
use super::super::records::CompletionRecord;
use super::identity::{WORKER_HANDOFF_CONTRACT, WORKER_HANDOFF_SCHEMA_DIGEST, WorkerTuple};

const TERMINAL_MAX_BYTES: usize = 16_384;
const SYNTHETIC_REQUEST_ID: &str = "00000000-0000-4000-8000-000000000000";
const SYNTHETIC_WATCHDOG_BOOT_ID: &str = "00000000-0000-4000-8000-000000000001";
const SYNTHETIC_WORKER_BOOT_ID: &str = "00000000-0000-4000-8000-000000000002";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerHandoffState {
    Admitted,
    Running,
    Unknown,
    Terminal,
    Acknowledged,
}

impl WorkerHandoffState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Running => "running",
            Self::Unknown => "unknown",
            Self::Terminal => "terminal",
            Self::Acknowledged => "acknowledged",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "admitted" => Self::Admitted,
            "running" => Self::Running,
            "unknown" => Self::Unknown,
            "terminal" => Self::Terminal,
            "acknowledged" => Self::Acknowledged,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerReservationState {
    Reserved,
    Unknown,
}

impl WorkerReservationState {
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "reserved" => Self::Reserved,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCompletionStatus {
    Completed,
    Failed,
}

impl WorkerCompletionStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    fn terminal_status(self) -> TerminalStatus {
        match self {
            Self::Completed => TerminalStatus::Completed,
            Self::Failed => TerminalStatus::Failed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerTerminalReceipt {
    pub tuple: WorkerTuple,
    pub status: WorkerCompletionStatus,
    pub checkpoint_sequence: u64,
    pub terminal_ref: String,
    pub result_digest: String,
    canonical: Vec<u8>,
}

impl WorkerTerminalReceipt {
    pub fn new(
        tuple: WorkerTuple,
        status: WorkerCompletionStatus,
        checkpoint_sequence: u64,
        terminal_ref: impl Into<String>,
        result_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        tuple.validate()?;
        let request = synthetic_request(&tuple)?;
        let terminal = TerminalRecord::new(
            &request,
            TerminalCompletion {
                status: status.terminal_status(),
                checkpoint_sequence,
                terminal_ref: terminal_ref.into(),
                result_digest: result_digest.into(),
            },
        )
        .map_err(|_| ExecutionStoreError::InvalidCompletion)?;
        Self::from_terminal(tuple, &terminal)
    }

    /// Converts the canonical worker-handoff terminal; no private digest is recomputed here.
    pub fn from_terminal(
        tuple: WorkerTuple,
        terminal: &TerminalRecord,
    ) -> Result<Self, ExecutionStoreError> {
        tuple.validate()?;
        let request = synthetic_request(&tuple)?;
        if !terminal.matches(&request) {
            return Err(ExecutionStoreError::Conflict);
        }
        let canonical = terminal
            .encode()
            .map_err(|_| ExecutionStoreError::InvalidCompletion)?;
        if canonical.len() > TERMINAL_MAX_BYTES {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        let fields: Value = serde_json::from_slice(&canonical)
            .map_err(|_| ExecutionStoreError::InvalidCompletion)?;
        let Value::Object(fields) = fields else {
            return Err(ExecutionStoreError::InvalidCompletion);
        };
        let status = WorkerCompletionStatus::from_str(
            fields
                .get("status")
                .and_then(Value::as_str)
                .ok_or(ExecutionStoreError::InvalidCompletion)?,
        )
        .ok_or(ExecutionStoreError::InvalidCompletion)?;
        let checkpoint_sequence = fields
            .get("checkpoint_sequence")
            .and_then(Value::as_u64)
            .ok_or(ExecutionStoreError::InvalidCompletion)?;
        let terminal_ref = fields
            .get("terminal_ref")
            .and_then(Value::as_str)
            .ok_or(ExecutionStoreError::InvalidCompletion)?
            .to_owned();
        let result_digest = fields
            .get("result_digest")
            .and_then(Value::as_str)
            .ok_or(ExecutionStoreError::InvalidCompletion)?
            .to_owned();
        if !valid_reference(&terminal_ref) {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        Ok(Self {
            tuple,
            status,
            checkpoint_sequence,
            terminal_ref,
            result_digest,
            canonical,
        })
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        self.tuple.validate()?;
        if !valid_reference(&self.terminal_ref) {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        let terminal = self.terminal_record()?;
        let canonical = terminal
            .encode()
            .map_err(|_| ExecutionStoreError::InvalidCompletion)?;
        if canonical != self.canonical {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        let rebuilt = Self::from_terminal(self.tuple.clone(), &terminal)?;
        if rebuilt.status != self.status
            || rebuilt.checkpoint_sequence != self.checkpoint_sequence
            || rebuilt.terminal_ref != self.terminal_ref
            || rebuilt.result_digest != self.result_digest
        {
            return Err(ExecutionStoreError::InvalidCompletion);
        }
        Ok(())
    }

    pub fn terminal_record(&self) -> Result<TerminalRecord, ExecutionStoreError> {
        TerminalRecord::decode(&self.canonical).map_err(|_| ExecutionStoreError::InvalidCompletion)
    }

    /// Uses the canonical WHJ terminal hash, including all tuple and result fields.
    pub fn acknowledgment_digest(&self) -> Result<String, ExecutionStoreError> {
        self.terminal_record()?
            .acknowledgment_digest()
            .map_err(|_| ExecutionStoreError::InvalidCompletion)
    }

    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredWorkerHandoff {
    pub tuple: WorkerTuple,
    pub worker_boot_id: String,
    pub watchdog_boot_id: String,
    pub mode_sequence: u64,
    pub state: WorkerHandoffState,
    pub reservation_state: WorkerReservationState,
    pub terminal: Option<WorkerTerminalReceipt>,
    pub acknowledged: bool,
    pub acknowledgment_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerLookup {
    Unknown { tuple: Box<WorkerTuple> },
    Known(Box<StoredWorkerHandoff>),
}

pub(crate) fn worker_terminal_from_completion(
    tuple: &WorkerTuple,
    completion: &CompletionRecord,
) -> Result<WorkerTerminalReceipt, ExecutionStoreError> {
    if completion.lineage != tuple.lineage()? {
        return Err(ExecutionStoreError::Conflict);
    }
    let status = match completion.status {
        CompletionStatus::Completed => WorkerCompletionStatus::Completed,
        CompletionStatus::Failed => WorkerCompletionStatus::Failed,
        CompletionStatus::Quarantined => return Err(ExecutionStoreError::Conflict),
    };
    WorkerTerminalReceipt::new(
        tuple.clone(),
        status,
        completion.checkpoint_sequence,
        completion.terminal_ref.clone(),
        completion.result_digest.clone(),
    )
}

fn synthetic_request(tuple: &WorkerTuple) -> Result<WorkerRequest, ExecutionStoreError> {
    let request = json!({
        "contract": WORKER_HANDOFF_CONTRACT,
        "schema_digest": WORKER_HANDOFF_SCHEMA_DIGEST,
        "direction": "request",
        "command": "dispatch",
        "scope": "dispatch",
        "request_id": SYNTHETIC_REQUEST_ID,
        "timeout_ms": 1,
        "watchdog_boot_id": SYNTHETIC_WATCHDOG_BOOT_ID,
        "worker_boot_id": SYNTHETIC_WORKER_BOOT_ID,
        "mode_sequence": 1,
        "operation": "runtime_v3_episode",
        "parameters": {},
        "handoff_id": tuple.handoff_id,
        "deployment_id": tuple.deployment_id,
        "job_id": tuple.job_id,
        "attempt_id": tuple.attempt_id,
        "attempt_number": tuple.attempt_number,
        "worker_owner_id": tuple.worker_owner_id,
        "worker_profile_digest": tuple.worker_profile_digest,
        "run_id": tuple.run_id,
        "episode_id": tuple.episode_id,
        "trajectory_id": tuple.trajectory_id,
        "payload_digest": tuple.payload_digest,
    });
    let bytes = serde_json::to_vec(&request).map_err(|_| ExecutionStoreError::InvalidCompletion)?;
    WorkerRequest::decode(&bytes).map_err(|_| ExecutionStoreError::InvalidCompletion)
}
