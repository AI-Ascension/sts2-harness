// SPDX-License-Identifier: MIT

mod action_envelope;
mod schema;
mod store_checkpoint;
mod store_completion;
mod store_core;
mod store_core_helpers;
mod store_jobs;
mod store_operation_queries;
mod store_ops;
mod store_provider;
mod store_provider_queries;
mod store_provider_results;
mod store_recovery;
mod store_recovery_attempt;
mod store_recovery_disposition;
mod store_worker;
mod store_worker_completion;
mod store_worker_control;
mod store_worker_lookup;
mod store_worker_queries;
mod types;

pub use store_core::ExecutionStore;
pub use types::{
    AttemptKind, AttemptState, CatalogEvidence, Checkpoint, CompletionRecord, CompletionStatus,
    DecisionReference, ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig,
    ExecutionStoreError, JobClaim, JobClaimOutcome, JobState, MAX_CATALOG_BYTES,
    MAX_OPERATION_ACTION_BYTES, MAX_ORIGINAL_CONTEXT_BYTES, OperationIntent, OperationState,
    ProviderFailureClass, ProviderReservation, ProviderReservationState, RECOVERY_CONTRACT_VERSION,
    RECOVERY_SCHEMA_DIGEST, RecoveryDisposition, ResumeState, StorePragmas, StoredAttempt,
    StoredDecision, StoredEpisode, StoredJob, StoredOperation, StoredWorkerHandoff,
    WORKER_EMPTY_PARAMETERS_DIGEST, WORKER_HANDOFF_CONTRACT, WORKER_HANDOFF_SCHEMA_DIGEST,
    WORKER_MAX_ATTEMPT_NUMBER, WorkerAdmissionContext, WorkerBoot, WorkerCompletionStatus,
    WorkerControlMode, WorkerControlRequest, WorkerControlState, WorkerHandoffState, WorkerLookup,
    WorkerOwnerProof, WorkerReservationState, WorkerTerminalReceipt, WorkerTuple,
};
