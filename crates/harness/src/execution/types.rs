// SPDX-License-Identifier: MIT

#[path = "types_checkpoint.rs"]
mod checkpoint;
#[path = "types_core.rs"]
mod core;
#[path = "types_enums.rs"]
mod enums;
#[path = "types_error.rs"]
mod error;
#[path = "types_records.rs"]
mod records;
#[path = "types_worker.rs"]
mod worker;

pub use checkpoint::{CatalogEvidence, Checkpoint};
pub use core::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig, MAX_CATALOG_BYTES,
    MAX_ORIGINAL_CONTEXT_BYTES, RECOVERY_CONTRACT_VERSION, RECOVERY_SCHEMA_DIGEST, StorePragmas,
};
pub use enums::{
    AttemptKind, AttemptState, CompletionStatus, JobState, OperationState, ProviderFailureClass,
    ProviderReservationState, RecoveryDisposition,
};
pub use error::ExecutionStoreError;
pub use records::{
    CompletionRecord, DecisionReference, JobClaim, JobClaimOutcome, MAX_OPERATION_ACTION_BYTES,
    OperationIntent, ProviderReservation, ResumeState, StoredAttempt, StoredDecision,
    StoredEpisode, StoredJob, StoredOperation,
};
pub use worker::{
    StoredWorkerHandoff, WORKER_EMPTY_PARAMETERS_DIGEST, WORKER_HANDOFF_CONTRACT,
    WORKER_HANDOFF_SCHEMA_DIGEST, WORKER_MAX_ATTEMPT_NUMBER, WorkerAdmissionContext, WorkerBoot,
    WorkerCompletionStatus, WorkerControlMode, WorkerControlRequest, WorkerControlState,
    WorkerHandoffState, WorkerLookup, WorkerOwnerProof, WorkerReservationState,
    WorkerTerminalReceipt, WorkerTuple,
};

pub(crate) use core::{valid_digest, valid_id, valid_original_context_raw, valid_reference};
pub(crate) use worker::valid_worker_existing_id;
pub(crate) use worker::{
    valid_worker_identity, valid_worker_uuid4, worker_terminal_from_completion,
};
