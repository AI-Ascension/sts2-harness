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

use std::sync::Arc;

pub use checkpoint::{CatalogEvidence, Checkpoint};
pub use core::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig, MAX_CATALOG_BYTES,
    RECOVERY_CONTRACT_VERSION, RECOVERY_SCHEMA_DIGEST, StorePragmas,
};
pub use enums::{
    AttemptKind, AttemptState, CompletionStatus, JobState, OperationState, ProviderFailureClass,
    ProviderReservationState, RecoveryDisposition,
};
pub use error::ExecutionStoreError;
pub use records::{
    CompletionRecord, DecisionReference, JobClaim, JobClaimOutcome, MAX_OPERATION_ACTION_BYTES,
    MAX_ORIGINAL_CONTEXT_BYTES, OperationIntent, ProviderReservation, ResumeState, StoredAttempt,
    StoredDecision, StoredEpisode, StoredJob, StoredOperation,
};
pub use worker::{
    StoredWorkerHandoff, WORKER_EMPTY_PARAMETERS_DIGEST, WORKER_HANDOFF_CONTRACT,
    WORKER_HANDOFF_SCHEMA_DIGEST, WORKER_MAX_ATTEMPT_NUMBER, WorkerAdmissionContext,
    WorkerAdmissionOutcome, WorkerBoot, WorkerCompletionStatus, WorkerControlMode,
    WorkerControlRequest, WorkerControlState, WorkerExecutionPermit, WorkerHandoffState,
    WorkerLookup, WorkerOwnerProof, WorkerReservationState, WorkerTerminalReceipt, WorkerTuple,
};

pub(crate) use core::{valid_digest, valid_id, valid_reference};
pub(crate) use worker::{
    valid_worker_existing_id, valid_worker_identity, valid_worker_uuid4,
    worker_terminal_from_completion,
};

pub(super) fn issue_worker_execution_permit(
    handoff_id: String,
    owner: &Arc<()>,
) -> WorkerExecutionPermit {
    WorkerExecutionPermit::new(handoff_id, owner)
}

pub(super) fn worker_execution_permit_belongs_to(
    permit: &WorkerExecutionPermit,
    owner: &Arc<()>,
) -> bool {
    permit.belongs_to(owner)
}

pub(super) fn consume_worker_execution_permit(permit: WorkerExecutionPermit) -> String {
    permit.handoff_id()
}
